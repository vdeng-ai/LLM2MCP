//! Immutable, UTF-8-safe pages of a validated complete result.
use crate::workspace::{estimate_tokens, truncate_tokens_strict};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

pub fn page(job_id: &str, text: &str, cursor: Option<&str>, budget: usize) -> Result<Value> {
    if !(256..=8192).contains(&budget) {
        bail!("page_tokens must be between 256 and 8192");
    }
    let hash = digest(text);
    let offset = if let Some(cursor) = cursor {
        let (expected, tail) = cursor.split_once(':').context("invalid result cursor")?;
        let (expected_job, offset) = tail.split_once(':').context("invalid result cursor")?;
        if expected_job != job_id {
            bail!("result cursor belongs to a different job");
        }
        if expected != hash {
            bail!("result cursor belongs to a different result");
        }
        offset.parse::<usize>().context("invalid result offset")?
    } else {
        0
    };
    if offset > text.len() || !text.is_char_boundary(offset) {
        bail!("invalid UTF-8 result offset");
    }
    // The envelope and escaped JSON text are included in the page budget.
    let mut allowance = budget;
    loop {
        let content = truncate_tokens_strict(&text[offset..], allowance, "");
        let end = offset + content.len();
        let value = json!({"format":"llm2mcp-result-page-v1", "job_id":job_id,
            "sha256":hash, "total_bytes":text.len(), "offset":offset, "end":end,
            "text":content, "next_cursor":(end < text.len()).then(|| format!("{hash}:{job_id}:{end}")),
            "complete":end == text.len()});
        let cost = estimate_tokens(&value.to_string());
        if cost <= budget {
            if end == offset && end < text.len() {
                bail!("page budget cannot fit result envelope; increase page_tokens");
            }
            return Ok(value);
        }
        if allowance == 0 {
            bail!("page budget cannot fit result envelope; increase page_tokens");
        }
        allowance = allowance.saturating_sub((cost - budget).max(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_pages_reconstruct_exactly_and_stay_in_budget() {
        let text = "中文🙂\r\n\\\"Hello world\"\n".repeat(500);
        let mut cursor = None;
        let mut reconstructed = String::new();
        loop {
            let value = page("job_test", &text, cursor.as_deref(), 256).unwrap();
            assert!(estimate_tokens(&value.to_string()) <= 256);
            reconstructed.push_str(value["text"].as_str().unwrap());
            cursor = value["next_cursor"].as_str().map(str::to_owned);
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(reconstructed, text);
    }
    #[test]
    fn rejects_foreign_malformed_and_non_boundary_cursors() {
        let text = "中文";
        let hash = digest(text);
        for cursor in [
            "../x".to_owned(),
            format!("{}:0", digest("other")),
            format!("{hash}:job:1"),
            format!("{hash}:job:999"),
        ] {
            assert!(page("job", text, Some(&cursor), 256).is_err());
        }
        assert!(page("job", text, None, 255).is_err());
        assert_eq!(page("job", "", None, 256).unwrap()["complete"], true);
    }
}
