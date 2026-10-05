use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;
static ASSIGNMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?im)(["']?(?:api[_-]?key|access[_-]?token|refresh[_-]?token|token|password|passwd|client[_-]?secret|secret|authorization)["']?\s*[:=]\s*)("[^"\r\n]*"|'[^'\r\n]*'|[^\s"'`,;]+)"#).expect("credential regex")
});
static BEARER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(Bearer\s+)[A-Za-z0-9._~+/=-]{8,}").expect("bearer regex"));
static KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:sk-[A-Za-z0-9_-]{16,}|AKIA[A-Z0-9]{16}|gh[pousr]_[A-Za-z0-9]{20,})\b")
        .expect("key regex")
});
static PRIVATE_KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----")
        .expect("private key regex")
});
pub fn redact(text: &str) -> String {
    let text = PRIVATE_KEY.replace_all(text, |caps: &regex::Captures<'_>| {
        format!(
            "[REDACTED PRIVATE KEY]{}",
            "\n".repeat(caps[0].bytes().filter(|byte| *byte == b'\n').count())
        )
    });
    let text = BEARER.replace_all(&text, "${1}[REDACTED]");
    let text = ASSIGNMENT.replace_all(&text, |caps: &regex::Captures<'_>| {
        let quote = if caps[2].starts_with('"') {
            "\""
        } else if caps[2].starts_with('\'') {
            "'"
        } else {
            ""
        };
        format!("{}{quote}[REDACTED]{quote}", &caps[1])
    });
    KEY.replace_all(&text, "[REDACTED]").into_owned()
}
pub fn redact_value(value: Value) -> Value {
    match value {
        Value::String(text) => Value::String(redact(&text)),
        Value::Array(items) => Value::Array(items.into_iter().map(redact_value).collect()),
        Value::Object(items) => Value::Object(
            items
                .into_iter()
                .map(|(key, value)| {
                    let name = key.to_ascii_lowercase().replace('-', "_");
                    let value = if [
                        "api_key",
                        "apikey",
                        "password",
                        "passwd",
                        "secret",
                        "client_secret",
                        "token",
                        "access_token",
                        "refresh_token",
                        "authorization",
                    ]
                    .contains(&name.as_str())
                    {
                        Value::String("[REDACTED]".to_owned())
                    } else {
                        redact_value(value)
                    };
                    (key, value)
                })
                .collect(),
        ),
        value => value,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn secrets_are_removed_and_quotes_and_lines_preserved() {
        let source = "API_KEY=FAKE_SECRET\nAuthorization: Bearer abcdef12345678\nlet password = \"multi word secret\";\n";
        let clean = redact(source);
        for secret in ["FAKE_SECRET", "abcdef12345678", "multi word secret"] {
            assert!(!clean.contains(secret));
        }
        assert_eq!(clean.lines().count(), source.lines().count());
        assert!(clean.contains("password = \"[REDACTED]\";"));
        assert_eq!(
            redact_value(serde_json::json!({"nested":{"api_key":"unknown-format"}}))["nested"]["api_key"],
            "[REDACTED]"
        );
    }
}
