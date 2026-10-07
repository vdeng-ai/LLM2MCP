pub fn keywords(task: &str) -> Vec<String> {
    let mut words = task
        .split(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .filter(|word| word.chars().count() >= 2)
        .map(str::to_lowercase)
        .collect::<Vec<_>>();
    for (chinese, aliases) in [
        ("登录", "login auth session"),
        ("认证", "auth authentication"),
        ("权限", "permission authorization"),
        ("缓存", "cache"),
        ("任务", "job task worker"),
        ("队列", "queue scheduler"),
        ("并发", "concurrency mutex lock"),
        ("取消", "cancel cancellation"),
        ("模型", "model llm"),
        ("请求", "request http client"),
        ("配置", "config settings"),
        ("数据库", "database sql"),
        ("文档", "docs document"),
        ("更新", "update updater"),
        ("支付", "payment checkout"),
        ("上传", "upload"),
        ("下载", "download"),
        ("超时", "timeout"),
        ("重试", "retry"),
        ("文件", "file filesystem"),
        ("解析", "parse parser"),
        ("日志", "log logging"),
        ("界面", "gui ui"),
        ("网络", "network http"),
    ] {
        if task.contains(chinese) {
            words.extend(aliases.split_whitespace().map(str::to_owned));
        }
    }
    words.retain(|word| {
        ![
            "the",
            "and",
            "for",
            "with",
            "from",
            "this",
            "that",
            "what",
            "why",
            "how",
            "find",
            "fix",
            "plan",
            "analyze",
            "review",
            "implement",
            "code",
            "project",
            "issue",
            "problem",
            "need",
            "should",
        ]
        .contains(&word.as_str())
    });
    words.sort();
    words.dedup();
    words
}
#[derive(Clone, Copy)]
enum Folding {
    Unicode,
    Ascii,
}

/// Compile once per query. Count each keyword once even when matches overlap
/// or repeat, preserving the previous substring scores and weighted previews.
pub struct KeywordMatcher {
    words: Vec<String>,
    weights: Vec<usize>,
    unit_weights: bool,
    automaton: Option<aho_corasick::AhoCorasick>,
    ascii_insensitive: bool,
    folding: Folding,
    seen: Vec<u32>,
    epoch: u32,
    total: usize,
}

impl KeywordMatcher {
    pub fn new(words: &[String]) -> Self {
        Self::weighted(words, |_| 1)
    }
    pub fn ascii(words: &[String]) -> Self {
        let mut matcher = Self::new(words);
        matcher.folding = Folding::Ascii;
        matcher
    }
    pub fn weighted(words: &[String], weight: impl Fn(&str) -> usize) -> Self {
        let weights = words.iter().map(|word| weight(word)).collect::<Vec<_>>();
        let ascii_insensitive = words
            .iter()
            .all(|word| !word.bytes().any(|byte| byte.is_ascii_uppercase()));
        // The paired benchmark favors native literal search below 64 words.
        // Build failures also retain that path rather than failing a task.
        let automaton = (words.len() >= 64)
            .then(|| {
                aho_corasick::AhoCorasickBuilder::new()
                    .ascii_case_insensitive(ascii_insensitive)
                    .build(words)
                    .ok()
            })
            .flatten();
        Self {
            total: weights.iter().sum(),
            unit_weights: weights.iter().all(|weight| *weight == 1),
            weights,
            words: words.to_vec(),
            automaton,
            ascii_insensitive,
            folding: Folding::Unicode,
            seen: vec![0; words.len()],
            epoch: 0,
        }
    }

    pub fn score(&mut self, text: &str) -> usize {
        use std::borrow::Cow;
        if self.words.is_empty() {
            return 0;
        }
        let Some(automaton) = &self.automaton else {
            // Keep the original single folding pass for short queries. Extra
            // scans to avoid its allocation cost more on the paired corpus.
            let folded = match self.folding {
                Folding::Unicode => text.to_lowercase(),
                Folding::Ascii => text.to_ascii_lowercase(),
            };
            if self.unit_weights {
                return self
                    .words
                    .iter()
                    .filter(|word| folded.contains(word.as_str()))
                    .count();
            }
            return self
                .words
                .iter()
                .zip(&self.weights)
                .filter(|(word, _)| folded.contains(word.as_str()))
                .map(|(_, weight)| *weight)
                .sum();
        };
        let unicode = matches!(self.folding, Folding::Unicode) && !text.is_ascii();
        let folded = if unicode {
            Cow::Owned(text.to_lowercase())
        } else if self.ascii_insensitive {
            Cow::Borrowed(text)
        } else {
            Cow::Owned(text.to_ascii_lowercase())
        };
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            self.seen.fill(0);
            self.epoch = 1;
        }
        let mut score = 0;
        for found in automaton.find_overlapping_iter(folded.as_bytes()) {
            let index = found.pattern().as_usize();
            if self.seen[index] != self.epoch {
                self.seen[index] = self.epoch;
                score += self.weights[index];
                if score == self.total {
                    break;
                }
            }
        }
        score
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn legacy_score(words: &[String], text: &str, ascii: bool, weighted: bool) -> usize {
        let text = if ascii {
            text.to_ascii_lowercase()
        } else {
            text.to_lowercase()
        };
        words
            .iter()
            .filter(|word| text.contains(word.as_str()))
            .map(|word| if weighted && word.contains('_') { 4 } else { 1 })
            .sum()
    }

    #[test]
    fn native_and_automaton_scores_preserve_overlaps_duplicates_and_unicode_folding() {
        let base = [
            "he",
            "she",
            "hers",
            "cache",
            "ache",
            "登录",
            "worker_timeout",
            "auth",
            "session",
            "ος",
            "i\u{307}",
            "k",
        ];
        for count in [0, 3, 8, 12, 40, 64, 96] {
            let words = (0..count)
                .map(|index| base[index % base.len()].to_owned())
                .collect::<Vec<_>>();
            let mut unicode = KeywordMatcher::new(&words);
            let mut ascii = KeywordMatcher::ascii(&words);
            let mut weighted =
                KeywordMatcher::weighted(&words, |word| if word.contains('_') { 4 } else { 1 });
            for text in [
                "",
                "SHE HERS CACHECACHE",
                "登录 AUTH auth worker_TIMEOUT",
                "ΟΣ İ K",
                "unrelated",
                "she cache",
                "auth",
                "登录",
                "CACHE",
            ] {
                assert_eq!(
                    unicode.score(text),
                    legacy_score(&words, text, false, false),
                    "Unicode: {count}, {text}"
                );
                assert_eq!(
                    ascii.score(text),
                    legacy_score(&words, text, true, false),
                    "ASCII: {count}, {text}"
                );
                assert_eq!(
                    weighted.score(text),
                    legacy_score(&words, text, false, true),
                    "Weighted: {count}, {text}"
                );
            }
        }
    }

    #[test]
    fn empty_patterns_uppercase_patterns_and_epoch_rollover_do_not_change_scores() {
        let mut words = ["", "", "ABC", "abc", "cache", "ache", "she", "he"]
            .map(str::to_owned)
            .to_vec();
        words.extend((0..56).map(|index| format!("unused_{index}")));
        let mut matcher = KeywordMatcher::new(&words);
        assert!(matcher.automaton.is_some());
        for text in ["", "ABC CACHE SHE", "a", "cache"] {
            assert_eq!(
                matcher.score(text),
                legacy_score(&words, text, false, false)
            );
        }
        matcher.epoch = u32::MAX;
        matcher.seen.fill(1);
        assert_eq!(
            matcher.score("ABC CACHE SHE"),
            legacy_score(&words, "ABC CACHE SHE", false, false)
        );
        assert_eq!(matcher.epoch, 1);
        assert_eq!(matcher.score(""), 2);
    }

    #[test]
    fn chinese_intent_matches_identifiers() {
        assert!(
            super::KeywordMatcher::new(&super::keywords("分析登录缓存失效的根因"))
                .score("src/auth/session_cache.rs")
                >= 3
        );
    }
}
