//! Verify citation locations against the actual supplied source, never the manifest alone.
use regex::Regex;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub fn verify(mut value: Value, prompt: &str) -> Value {
    let source = prompt.split("SOURCE MATERIAL\n").nth(1).unwrap_or(prompt);
    let header = Regex::new(r"^===== FILE: (.+?)(?: \(SELECTED SYMBOLS\))? =====$").unwrap();
    let mut files = BTreeMap::<String, BTreeSet<usize>>::new();
    let mut current = None;
    for line in source.lines() {
        if let Some(capture) = header.captures(line) {
            let path = capture[1].to_owned();
            files.entry(path.clone()).or_default();
            current = Some(path);
        } else if line.starts_with("===== END FILE:") {
            current = None;
        } else if let Some(path) = &current
            && let Some((number, _)) = line.split_once(" | ")
            && let Ok(number) = number.trim().parse::<usize>()
        {
            files.entry(path.clone()).or_default().insert(number);
        }
    }
    // Diff batches carry explicit path and source wrappers even for deleted files.
    for line in source
        .lines()
        .filter(|line| line.starts_with("FILE ") && line.contains(" PART "))
    {
        if let Some((path, _)) = line.trim_start_matches("FILE ").split_once(" PART ") {
            files.entry(path.to_owned()).or_default();
        }
    }
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
                    let valid = citation.as_str().is_some_and(|citation| {
                        if let Some((path, range)) = citation.rsplit_once(':') {
                            let mut numbers = range.split('-');
                            if let Some(start) =
                                numbers.next().and_then(|n| n.parse::<usize>().ok())
                            {
                                let end = numbers
                                    .next()
                                    .and_then(|n| n.parse::<usize>().ok())
                                    .unwrap_or(start);
                                return start > 0
                                    && end >= start
                                    && end - start <= 10000
                                    && files.get(path).is_some_and(|lines| {
                                        (start..=end).all(|n| lines.contains(&n))
                                    });
                            }
                        }
                        files.contains_key(citation)
                    });
                    if valid {
                        accepted.push(citation);
                    } else {
                        rejected += 1;
                    }
                }
                let unverified = accepted.is_empty();
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
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_manifest_only_paths_and_unseen_line_ranges() {
        let prompt = "REPOSITORY MANIFEST\nfake.rs\nSOURCE MATERIAL\n===== FILE: src/a.rs =====\n10 | fn a() {}\n11 | }\n===== END FILE: src/a.rs =====\n";
        let value = json!({"findings":[{"text":"bug","evidence":["src/a.rs:10-11","src/a.rs:10-12","fake.rs","src/a.rs:999"]}]});
        let result = verify(value, prompt);
        assert_eq!(result["findings"][0]["evidence"], json!(["src/a.rs:10-11"]));
        assert!(
            result["findings"][0]["text"]
                .as_str()
                .unwrap()
                .contains("REMOVED")
        );
    }
}
