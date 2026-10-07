//! Compact validated analysis without letting verbose fields consume every slot.
use crate::workspace::{estimate_tokens, truncate_tokens_strict};
use serde_json::Value;

const TRUNCATED: &str = "[PRIMARY RESULT TRUNCATED]";

struct Section {
    heading: &'static str,
    lines: Vec<String>,
}

impl Section {
    fn text(&self) -> String {
        format!("{}\n{}", self.heading, self.lines.join("\n"))
    }
}

fn severity(item: &Value) -> usize {
    match item["severity"].as_str().unwrap_or("") {
        "critical" => 0,
        "high" => 1,
        "medium" => 2,
        "low" => 3,
        _ => 4,
    }
}

fn list(value: &Value, key: &str, limit: usize) -> (Vec<String>, bool) {
    let Some(items) = value.get(key).and_then(Value::as_array) else {
        return (Vec::new(), false);
    };
    let mut ranked = items.iter().collect::<Vec<_>>();
    if matches!(key, "findings" | "evidence") {
        ranked.sort_by_key(|item| severity(item));
    }
    let lines = ranked
        .into_iter()
        .take(limit)
        .filter_map(|item| match item {
            Value::String(text) => Some(format!("- {}", text.trim())),
            Value::Object(object) => {
                let text = object
                    .get("text")
                    .or_else(|| object.get("description"))
                    .and_then(Value::as_str)?;
                let severity = object
                    .get("severity")
                    .and_then(Value::as_str)
                    .map(|value| format!("[{value}] "))
                    .unwrap_or_default();
                let refs = object
                    .get("evidence")
                    .and_then(Value::as_array)
                    .map(|refs| {
                        refs.iter()
                            .filter_map(Value::as_str)
                            .take(3)
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .filter(|refs| !refs.is_empty())
                    .map(|refs| format!("({refs}) "))
                    .unwrap_or_default();
                Some(format!("- {severity}{refs}{}", text.trim()))
            }
            _ => None,
        })
        .collect();
    (lines, items.len() > limit)
}

fn bounded_sections(sections: &[Section], budget: usize) -> String {
    let separators = sections.len().saturating_sub(1) * 2;
    let per_section = budget.saturating_sub(separators) / sections.len().max(1);
    sections
        .iter()
        .map(|section| truncate_tokens_strict(&section.text(), per_section, "…"))
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub fn compact(value: &Value, read_next: &[String], budget: usize) -> String {
    if budget == 0 {
        return String::new();
    }
    let summary = [
        ("diagnosis", "DIAGNOSIS"),
        ("root_cause", "ROOT CAUSE"),
        ("conclusion", "CONCLUSION"),
        ("goal", "GOAL"),
        ("confidence", "CONFIDENCE"),
        ("intermittency", "INTERMITTENCY"),
    ]
    .into_iter()
    .filter_map(|(key, heading)| {
        value.get(key).and_then(Value::as_str).and_then(|text| {
            (!text.trim().is_empty()).then(|| Section {
                heading,
                lines: vec![text.trim().to_owned()],
            })
        })
    })
    .collect::<Vec<_>>();
    let mut omitted = false;
    let mut findings = Vec::new();
    let mut details = Vec::new();
    for (key, heading, limit, important) in [
        ("evidence", "EVIDENCE", 8, true),
        ("findings", "FINDINGS", 8, true),
        ("verification", "HOW TO VERIFY", 6, false),
        ("steps", "STEPS", 10, false),
        ("actions", "ACTIONS", 6, false),
        ("fix_area", "LIKELY FIX AREA", 6, false),
        ("execution_path", "EXECUTION PATH", 8, false),
        ("alternatives", "ALTERNATIVE HYPOTHESES", 5, false),
        ("risks", "RISKS", 5, false),
        ("tests", "TESTS", 6, false),
    ] {
        let (lines, clipped) = list(value, key, limit);
        omitted |= clipped;
        if !lines.is_empty() {
            let section = Section { heading, lines };
            if important {
                findings.push(section);
            } else {
                details.push(section);
            }
        }
    }
    let reads = if read_next.is_empty() {
        Vec::new()
    } else {
        vec![Section {
            heading: "READ_NEXT",
            lines: read_next.to_vec(),
        }]
    };
    let groups = [
        (30usize, summary),
        (45, findings),
        (15, reads),
        (10, details),
    ]
    .into_iter()
    .filter(|(_, sections)| !sections.is_empty())
    .collect::<Vec<_>>();
    if groups.is_empty() {
        return truncate_tokens_strict(&value.to_string(), budget, TRUNCATED);
    }
    let full = groups
        .iter()
        .flat_map(|(_, sections)| sections.iter().map(Section::text))
        .collect::<Vec<_>>()
        .join("\n\n");
    let full = if omitted {
        format!("{full}\n\n{TRUNCATED}")
    } else {
        full
    };
    if estimate_tokens(&full) <= budget {
        return full;
    }
    // Reserve each present group's share before rendering any verbose field.
    let usable = budget.saturating_sub(estimate_tokens(TRUNCATED) + groups.len() * 2);
    let weight: usize = groups.iter().map(|(weight, _)| weight).sum();
    let mut parts = groups
        .iter()
        .map(|(share, sections)| bounded_sections(sections, usable * share / weight))
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    parts.push(TRUNCATED.to_owned());
    truncate_tokens_strict(&parts.join("\n\n"), budget, TRUNCATED)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn verbose_summary_and_reads_cannot_displace_a_late_critical_finding() {
        let mut findings = (0..12)
            .map(|_| json!({"severity":"info", "text":"Unimportant detail", "evidence":[]}))
            .collect::<Vec<_>>();
        findings.push(
            json!({"severity":"critical", "text":"CRITICAL_REGRESSION", "evidence":["src/a.rs:1"]}),
        );
        let value = json!({"conclusion":"Summary ".repeat(2000), "findings":findings});
        let reads = vec![format!(
            "- src/a.rs:1-2 — {}",
            "Verbose reason ".repeat(2000)
        )];
        let result = compact(&value, &reads, 128);
        assert!(result.contains("CRITICAL_REGRESSION"));
        assert!(result.contains("src/a.rs:1-2"));
        assert!(result.contains("CONCLUSION"));
        assert!(result.contains(TRUNCATED));
        assert!(estimate_tokens(&result) <= 128);
    }

    #[test]
    fn preserves_complete_short_results_and_every_budget() {
        let value = json!({"conclusion":"OK", "findings":[{"severity":"high", "text":"Check worker", "evidence":[]}]});
        let full = compact(&value, &[], 1000);
        assert!(full.contains("Check worker"));
        assert!(!full.contains(TRUNCATED));
        for budget in 0..150 {
            assert!(estimate_tokens(&compact(&value, &[], budget)) <= budget);
        }
    }
}
