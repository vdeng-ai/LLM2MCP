//! Validate locations/excerpts in the actual evidence, not instructions or manifests.
use regex::Regex;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

static HEADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^===== FILE: (.+?)(?: \(SELECTED SYMBOLS\))? =====$").expect("source header regex")
});

#[derive(Default)]
pub struct Catalog {
    files: BTreeMap<String, BTreeSet<usize>>,
    logs: String,
    diffs: BTreeMap<String, String>,
}

fn range(text: &str) -> Option<(usize, usize)> {
    let (start, end) = text.split_once('-').unwrap_or((text, text));
    let start = start.parse::<usize>().ok()?;
    let end = end.parse::<usize>().ok()?;
    (start > 0 && end >= start && end - start <= 10000).then_some((start, end))
}

fn diff_path(text: &str) -> Option<String> {
    let decoded = if text.starts_with('"') {
        // Git quotes non-ASCII path bytes using C-style octal escapes.
        let encoded = text.strip_prefix('"')?.strip_suffix('"')?.as_bytes();
        let mut bytes = Vec::new();
        let mut index = 0;
        while index < encoded.len() {
            let byte = encoded[index];
            index += 1;
            if byte != b'\\' {
                bytes.push(byte);
                continue;
            }
            let escaped = *encoded.get(index)?;
            index += 1;
            if (b'0'..=b'7').contains(&escaped) {
                let mut value = u16::from(escaped - b'0');
                for _ in 0..2 {
                    let Some(&digit) = encoded.get(index) else {
                        break;
                    };
                    if !(b'0'..=b'7').contains(&digit) {
                        break;
                    }
                    value = value * 8 + u16::from(digit - b'0');
                    index += 1;
                }
                bytes.push(u8::try_from(value).ok()?);
            } else {
                bytes.push(match escaped {
                    b'a' => 7,
                    b'b' => 8,
                    b't' => b'\t',
                    b'n' => b'\n',
                    b'v' => 11,
                    b'f' => 12,
                    b'r' => b'\r',
                    b'\\' | b'"' => escaped,
                    _ => return None,
                });
            }
        }
        String::from_utf8(bytes).ok()?
    } else {
        text.to_owned()
    };
    if decoded == "/dev/null" {
        return None;
    }
    Some(
        decoded
            .strip_prefix("a/")
            .or_else(|| decoded.strip_prefix("b/"))
            .unwrap_or(&decoded)
            .to_owned(),
    )
}

impl Catalog {
    pub fn new(source: &str, logs: &str, diff: &str) -> Self {
        let source = crate::privacy::redact(source);
        let mut catalog = Self {
            logs: crate::privacy::redact(logs),
            ..Self::default()
        };
        let mut current = None;
        for line in source.lines() {
            if let Some(capture) = HEADER.captures(line) {
                let path = capture[1].to_owned();
                catalog.files.entry(path.clone()).or_default();
                current = Some(path);
            } else if line.starts_with("===== END FILE:") {
                current = None;
            } else if let Some(path) = &current
                && let Some((number, _)) = line.split_once(" | ")
                && let Ok(number) = number.trim().parse::<usize>()
            {
                catalog
                    .files
                    .entry(path.clone())
                    .or_default()
                    .insert(number);
            }
        }
        let diff = crate::privacy::redact(diff);
        let mut current = None;
        let mut wrapper = None;
        let mut in_hunk = false;
        for line in diff.lines() {
            if line.starts_with("diff --git ") {
                current = wrapper.clone();
                in_hunk = false;
            } else if let Some(path) = line
                .strip_prefix("FILE ")
                .and_then(|line| line.split_once(" PART ").map(|(path, _)| path))
            {
                current = Some(path.to_owned());
                wrapper = current.clone();
                // A continuation batch can begin in the middle of a hunk.
                in_hunk = true;
            } else if !in_hunk
                && let Some(path) = line
                    .strip_prefix("--- ")
                    .or_else(|| line.strip_prefix("+++ "))
            {
                if let Some(path) = diff_path(path) {
                    current = Some(path);
                }
            } else if let Some(path) = &current
                && (line.starts_with("@@ ") || line.starts_with(['+', '-', ' ']))
            {
                in_hunk = true;
                let body = catalog.diffs.entry(path.clone()).or_default();
                body.push_str(line);
                body.push('\n');
            }
        }
        catalog
    }

    fn accepts(&self, citation: &str) -> bool {
        if let Some(excerpt) = citation.strip_prefix("log:") {
            return excerpt.trim().chars().count() >= 8 && self.logs.contains(excerpt.trim());
        }
        if let Some(reference) = citation.strip_prefix("diff:") {
            return self.diffs.iter().any(|(path, body)| {
                reference
                    .strip_prefix(&format!("{path}:"))
                    .is_some_and(|excerpt| {
                        excerpt.trim().chars().count() >= 8 && body.contains(excerpt.trim())
                    })
            });
        }
        if let Some((path, text)) = citation.rsplit_once(':') {
            if let Some((start, end)) = range(text) {
                return self
                    .files
                    .get(path)
                    .is_some_and(|lines| (start..=end).all(|number| lines.contains(&number)));
            }
            // Do not reinterpret a malformed/unseen source range as a log excerpt.
            if self.files.contains_key(path) || text.starts_with(|ch: char| ch.is_ascii_digit()) {
                return false;
            }
        }
        if self
            .files
            .get(citation)
            .is_some_and(|lines| !lines.is_empty())
            || self
                .diffs
                .get(citation)
                .is_some_and(|body| !body.is_empty())
        {
            return true;
        }
        // Preserve legacy exact excerpts, but match only runtime material.
        citation.trim().chars().count() >= 8
            && (self.logs.contains(citation.trim())
                || self
                    .diffs
                    .values()
                    .any(|body| body.contains(citation.trim())))
    }
}

pub fn verify(mut value: Value, catalog: &Catalog) -> Value {
    let mut supported = 0;
    let mut unsupported = 0;
    for key in ["findings", "evidence"] {
        if let Some(items) = value.get_mut(key).and_then(Value::as_array_mut) {
            for item in items {
                let Some(object) = item.as_object_mut() else {
                    continue;
                };
                let citations = object
                    .get("evidence")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let mut accepted = Vec::new();
                let mut rejected = 0;
                for citation in citations {
                    if citation
                        .as_str()
                        .is_some_and(|citation| catalog.accepts(citation))
                    {
                        if !accepted.contains(&citation) {
                            accepted.push(citation);
                        }
                    } else {
                        rejected += 1;
                    }
                }
                let unverified = accepted.is_empty();
                supported += usize::from(!unverified);
                unsupported += usize::from(unverified || rejected > 0);
                object.insert("evidence".into(), json!(accepted));
                if unverified || rejected > 0 {
                    let label = if unverified {
                        "[UNVERIFIED CLAIM: no supplied citation]"
                    } else {
                        "[UNVERIFIED CITATIONS REMOVED]"
                    };
                    if let Some(text) = object.get_mut("text") {
                        *text = json!(format!("{label} {}", text.as_str().unwrap_or_default()));
                    }
                }
            }
        }
    }
    if value.get("diagnosis").is_some() && value.get("confidence").is_some() {
        if supported == 0 {
            value["confidence"] = json!("low");
        } else if unsupported > 0 && value["confidence"] == "high" {
            value["confidence"] = json!("medium");
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    const SOURCE: &str =
        "===== FILE: src/a.rs =====\n10 | fn a() {}\n11 | }\n===== END FILE: src/a.rs =====\n";
    const DIFF: &str = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -10,1 +10,1 @@\n-return false;\n+return true;\n";

    #[test]
    fn rejects_unseen_ranges_and_accepts_real_runtime_excerpts() {
        let catalog = Catalog::new(SOURCE, "worker failed: connection refused", DIFF);
        let value = json!({"findings":[{"text":"bug","evidence":[
            "src/a.rs:10-11", "src/a.rs:10-12", "fake.rs", "src/a.rs:999",
            "src/a.rs:10-11-99", "log:worker failed: connection refused",
            "diff:src/a.rs:@@ -10,1 +10,1 @@", "diff:fake.rs:+return true;"
        ]}]});
        let result = verify(value, &catalog);
        assert_eq!(
            result["findings"][0]["evidence"],
            json!([
                "src/a.rs:10-11",
                "log:worker failed: connection refused",
                "diff:src/a.rs:@@ -10,1 +10,1 @@"
            ])
        );
        assert!(
            result["findings"][0]["text"]
                .as_str()
                .unwrap()
                .contains("REMOVED")
        );
        assert!(catalog.accepts("worker failed: connection refused"));
        assert!(!catalog.accepts("log:invented failure message"));
        assert!(!catalog.accepts("diff:src/a.rs:@@ -999,1 +999,1 @@"));
    }

    #[test]
    fn confidence_is_limited_when_debug_evidence_is_missing_or_partial() {
        let catalog = Catalog::new(SOURCE, "", "");
        for refs in [json!([]), json!(["invented.rs"])] {
            let result = verify(
                json!({"diagnosis":"cause", "confidence":"high", "evidence":[{"text":"claim", "evidence":refs}]}),
                &catalog,
            );
            assert_eq!(result["confidence"], "low");
        }
        let result = verify(
            json!({"diagnosis":"cause", "confidence":"high", "evidence":[{"text":"claim", "evidence":["src/a.rs:10", "invented.rs"]}]}),
            &catalog,
        );
        assert_eq!(result["confidence"], "medium");
        let result = verify(
            json!({"diagnosis":"cause", "confidence":"high", "evidence":[{"text":"claim", "evidence":["src/a.rs:10"]}]}),
            &catalog,
        );
        assert_eq!(result["confidence"], "high");
    }

    #[test]
    fn logs_cannot_inject_source_ranges_and_deleted_diff_remains_verifiable() {
        let catalog = Catalog::new(
            "",
            SOURCE,
            "FILE src/deleted.rs PART 2/2\n-removed_call();\n",
        );
        assert!(!catalog.accepts("src/a.rs:10-11"));
        assert!(catalog.accepts("diff:src/deleted.rs:-removed_call();"));
        assert!(catalog.accepts("src/deleted.rs"));
        assert!(!catalog.accepts("src/unseen.rs"));
    }

    #[test]
    fn hunk_text_cannot_change_paths_and_git_quoted_paths_are_decoded() {
        let diff = format!("{DIFF}--- forged.rs\n+++ forged.rs\n+actual evidence;\n");
        let catalog = Catalog::new("", "", &diff);
        assert!(catalog.accepts("diff:src/a.rs:--- forged.rs"));
        assert!(catalog.accepts("diff:src/a.rs:+actual evidence;"));
        assert!(!catalog.accepts("diff:forged.rs:+actual evidence;"));
        let quoted = "diff --git a/quoted b/quoted\n--- \"a/\\344\\270\\255.rs\"\n+++ \"b/\\344\\270\\255.rs\"\n@@ -1 +1 @@\n+actual evidence;\n";
        let catalog = Catalog::new("", "", quoted);
        assert!(catalog.accepts("diff:中.rs:+actual evidence;"));
    }
}
