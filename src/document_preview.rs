//! Select exact Markdown fragments near changed identifiers within a fixed budget.
use crate::doc_edits::Fragment;
use crate::workspace::{estimate_tokens, truncate_tokens_strict};

fn changed_keywords(diff: &str) -> Vec<String> {
    let changed = diff
        .lines()
        .filter(|line| {
            (line.starts_with(['+', '-']) && !line.starts_with("+++") && !line.starts_with("---"))
                || line.starts_with("diff --git ")
                || line.starts_with("@@ ")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut keywords = crate::search::keywords(&changed);
    keywords.retain(|word| {
        word.chars().count() >= 3
            && !word.chars().all(|ch| ch.is_ascii_digit())
            && ![
                "let", "const", "pub", "mut", "self", "return", "true", "false", "none", "some",
                "string", "str", "usize", "u32", "u64", "bool", "fn", "impl", "use", "diff", "git",
                "src", "old", "new", "value", "name", "null", "int", "void",
            ]
            .contains(&word.as_str())
    });
    keywords.sort_by_key(|word| std::cmp::Reverse(word.len()));
    keywords.truncate(256);
    keywords
}

fn score(line: &str, keywords: &[String]) -> usize {
    let line = line.to_lowercase();
    keywords
        .iter()
        .filter(|word| line.contains(word.as_str()))
        .map(|word| if word.contains('_') { 4 } else { 1 })
        .sum()
}

fn sections(lines: &[&str]) -> Vec<(usize, usize)> {
    let mut starts = vec![0];
    let mut fence: Option<(char, usize)> = None;
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        let first = trimmed.chars().next().unwrap_or_default();
        if matches!(first, '`' | '~') {
            let width = trimmed.chars().take_while(|ch| *ch == first).count();
            if width >= 3 {
                if let Some((kind, length)) = fence {
                    if first == kind && width >= length && trimmed[width..].trim().is_empty() {
                        fence = None;
                    }
                } else {
                    fence = Some((first, width));
                }
                continue;
            }
        }
        if fence.is_some() {
            continue;
        }
        let level = trimmed.chars().take_while(|ch| *ch == '#').count();
        if (1..=6).contains(&level)
            && trimmed[level..].starts_with(char::is_whitespace)
            && index > 0
        {
            starts.push(index);
        }
    }
    starts
        .iter()
        .enumerate()
        .map(|(index, start)| {
            (
                *start,
                starts.get(index + 1).copied().unwrap_or(lines.len()),
            )
        })
        .collect()
}

fn fragment(lines: &[&str], start: usize, end: usize, budget: usize) -> Fragment {
    Fragment {
        start_line: start + 1,
        end_line: end,
        text: truncate_tokens_strict(&lines[start..end].concat(), budget, ""),
    }
}

pub fn select(text: &str, diff: &str, budget: usize) -> (Vec<Fragment>, bool) {
    let lines = text.split_inclusive('\n').collect::<Vec<_>>();
    if text.is_empty() || budget == 0 {
        return (Vec::new(), !text.is_empty());
    }
    if estimate_tokens(text) <= budget {
        return (vec![fragment(&lines, 0, lines.len(), budget)], false);
    }
    let mut prefix_tokens = vec![0usize];
    for line in &lines {
        prefix_tokens.push(prefix_tokens.last().copied().unwrap_or(0) + estimate_tokens(line));
    }
    let span_tokens = |start: usize, end: usize| prefix_tokens[end] - prefix_tokens[start];
    let keywords = changed_keywords(diff);
    let mut candidates = sections(&lines)
        .into_iter()
        .map(|(start, end)| {
            let (focus, body_score) = (start..end)
                .map(|index| (index, score(lines[index], &keywords)))
                .max_by_key(|(index, score)| (*score, std::cmp::Reverse(*index)))
                .unwrap_or((start, 0));
            (
                score(lines[start], &keywords) * 3 + body_score * 2,
                start,
                end,
                focus,
            )
        })
        .filter(|(score, _, _, _)| *score > 0)
        .collect::<Vec<_>>();
    candidates.sort_by_key(|(score, start, _, _)| (std::cmp::Reverse(*score), *start));
    candidates.truncate(4);
    if candidates.is_empty() {
        let preview = truncate_tokens_strict(text, budget, "");
        let end_line = preview.split_inclusive('\n').count();
        return (
            vec![Fragment {
                start_line: 1,
                end_line,
                text: preview,
            }],
            true,
        );
    }
    let per_section = budget / candidates.len();
    let mut selected = Vec::new();
    for (_, section_start, section_end, focus) in candidates {
        if span_tokens(section_start, section_end) <= per_section {
            selected.push(fragment(&lines, section_start, section_end, per_section));
            continue;
        }
        // Keep a section heading separately when the changed identifier is late.
        let heading = (focus > section_start).then(|| {
            fragment(
                &lines,
                section_start,
                section_start + 1,
                (per_section / 5).min(64),
            )
        });
        let heading_cost = heading
            .as_ref()
            .map(|heading| estimate_tokens(&heading.text))
            .unwrap_or(0);
        let body_budget = per_section.saturating_sub(heading_cost);
        let (mut start, mut end) = (focus, focus + 1);
        let lower_bound = section_start + usize::from(heading.is_some());
        loop {
            let previous = start > lower_bound && span_tokens(start - 1, end) <= body_budget;
            let next = end < section_end && span_tokens(start, end + 1) <= body_budget;
            if previous {
                start -= 1;
            } else if next {
                end += 1;
            } else {
                break;
            }
        }
        if let Some(heading) = heading
            && !heading.text.is_empty()
        {
            selected.push(heading);
        }
        let mut body = fragment(&lines, start, end, body_budget);
        if start == focus && end == focus + 1 && estimate_tokens(lines[focus]) > body_budget {
            // A single very long paragraph can also contain a late identifier.
            let anchor = keywords.iter().find_map(|word| {
                lines[focus].find(word).or_else(|| {
                    word.is_ascii()
                        .then(|| {
                            lines[focus]
                                .as_bytes()
                                .windows(word.len())
                                .position(|part| part.eq_ignore_ascii_case(word.as_bytes()))
                        })
                        .flatten()
                })
            });
            if let Some(anchor) = anchor {
                let mut begin = anchor.saturating_sub(80);
                while !lines[focus].is_char_boundary(begin) {
                    begin -= 1;
                }
                body.text = truncate_tokens_strict(&lines[focus][begin..], body_budget, "");
            }
        }
        if !body.text.is_empty() {
            selected.push(body);
        }
    }
    selected.sort_by_key(|fragment| fragment.start_line);
    (selected, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn late_related_section_replaces_irrelevant_document_prefix() {
        let text = format!(
            "# Introduction\n{}\n## Worker settings\nSet `worker_timeout_secs` to 30.\n\n## Appendix\nKEEP THIS APPENDIX\n",
            "Unrelated introduction.\n".repeat(400)
        );
        let (fragments, clipped) = select(
            &text,
            "-worker_timeout_secs = 30\n+worker_timeout_secs = 60\n",
            96,
        );
        assert!(clipped);
        assert!(
            fragments
                .iter()
                .any(|fragment| fragment.text.contains("worker_timeout_secs"))
        );
        assert!(
            !fragments
                .iter()
                .any(|fragment| fragment.text.contains("Unrelated introduction"))
        );
        assert!(
            fragments
                .iter()
                .map(|fragment| estimate_tokens(&fragment.text))
                .sum::<usize>()
                <= 96
        );
        assert!(
            fragments
                .iter()
                .all(|fragment| text.contains(&fragment.text))
        );
    }

    #[test]
    fn late_identifier_in_one_long_section_and_crlf_remain_exact() {
        let text = format!(
            "# 配置\r\n```rust\r\n## not a Markdown heading\r\n```\r\n{}worker_timeout_secs = 30\r\n",
            "无关的介绍。\r\n".repeat(300)
        );
        let (fragments, _) = select(&text, "+worker_timeout_secs = 60\n", 80);
        assert!(
            fragments
                .iter()
                .any(|fragment| fragment.text.contains("worker_timeout_secs = 30\r\n"))
        );
        assert!(
            fragments
                .iter()
                .all(|fragment| text.contains(&fragment.text))
        );
        assert_eq!(
            sections(&text.split_inclusive('\n').collect::<Vec<_>>()).len(),
            1
        );
    }

    #[test]
    fn unmatched_fallback_short_documents_and_long_paragraphs_are_bounded() {
        let (fragments, clipped) = select("# Small\nComplete\n", "", 100);
        assert!(!clipped);
        assert_eq!(fragments[0].text, "# Small\nComplete\n");
        let text = format!("{} worker_timeout_secs = 30", "Introduction ".repeat(1000));
        let (fragments, _) = select(&text, "+worker_timeout_secs = 60", 96);
        assert!(
            fragments
                .iter()
                .any(|fragment| fragment.text.contains("worker_timeout_secs"))
        );
        for budget in 0..100 {
            let (fragments, _) = select(&text, "+worker_timeout_secs = 60", budget);
            assert!(
                fragments
                    .iter()
                    .map(|fragment| estimate_tokens(&fragment.text))
                    .sum::<usize>()
                    <= budget
            );
        }
        let (fragments, _) = select(&text, "+unmatched_identifier = 1", 64);
        assert_eq!(fragments[0].start_line, 1);
        assert!(estimate_tokens(&fragments[0].text) <= 64);
        let unicode = format!("{} worker_timeout_secs = 30", "中文说明".repeat(1000));
        let (fragments, _) = select(&unicode, "+worker_timeout_secs = 60", 96);
        assert!(
            fragments
                .iter()
                .any(|fragment| fragment.text.contains("worker_timeout_secs"))
        );
        assert!(
            fragments
                .iter()
                .all(|fragment| unicode.contains(&fragment.text))
        );
    }
}
