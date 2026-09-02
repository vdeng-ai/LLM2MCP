use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::time::Duration;

use crate::config::{AppConfig, ReasoningEffort, ReasoningTransport};

#[derive(Debug, Clone)]
pub struct ChatResponse {
    pub content: Option<String>,
    pub reasoning_content: Option<String>,
    pub finish_reason: Option<String>,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
}

impl ChatResponse {
    pub fn final_text(&self) -> Option<&str> {
        self.content
            .as_deref()
            .filter(|value| !value.trim().is_empty())
    }

    pub fn diagnostics(&self) -> String {
        format!(
            "finish_reason={}, prompt_tokens={}, completion_tokens={}, reasoning_chars={}, content_chars={}",
            self.finish_reason.as_deref().unwrap_or("unknown"),
            self.prompt_tokens
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_owned()),
            self.completion_tokens
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_owned()),
            self.reasoning_content
                .as_deref()
                .map(|value| value.chars().count())
                .unwrap_or(0),
            self.content
                .as_deref()
                .map(|value| value.chars().count())
                .unwrap_or(0),
        )
    }

    pub fn exhausted_before_final(&self) -> bool {
        self.final_text().is_none()
            && (self.finish_reason.as_deref() == Some("length")
                || self
                    .reasoning_content
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty()))
    }
}

pub fn test_connection(config: &AppConfig) -> Result<String> {
    let client = client(config)?;
    let url = format!("{}/models", config.base_url.trim_end_matches('/'));
    let mut request = client.get(url);
    if !config.api_key.trim().is_empty() {
        request = request.bearer_auth(config.api_key.trim());
    }
    let response = request.send().context("failed to connect to LLM API")?;
    if !response.status().is_success() {
        bail!("LLM API returned HTTP {}", response.status());
    }
    Ok(format!("Connected: {}", config.model))
}

fn client(config: &AppConfig) -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(config.timeout_secs.max(5)))
        .build()
        .context("failed to build HTTP client")
}

pub fn chat(
    config: &AppConfig,
    system: &str,
    user: &str,
    effort: ReasoningEffort,
    max_output_tokens: u32,
) -> Result<String> {
    let response = chat_detailed(config, system, user, effort, max_output_tokens)?;
    response
        .final_text()
        .map(|value| value.trim().to_owned())
        .with_context(|| {
            format!(
                "LLM returned no final message content ({})",
                response.diagnostics()
            )
        })
}

pub fn chat_detailed(
    config: &AppConfig,
    system: &str,
    user: &str,
    effort: ReasoningEffort,
    max_output_tokens: u32,
) -> Result<ChatResponse> {
    let client = client(config)?;
    let mut payload = json!({
        "model": config.model,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user}
        ],
        "temperature": config.temperature,
        "max_tokens": max_output_tokens,
        "stream": false
    });
    apply_reasoning(&mut payload, config.reasoning_transport, effort);

    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
    let mut request = client.post(url).json(&payload);
    if !config.api_key.trim().is_empty() {
        request = request.bearer_auth(config.api_key.trim());
    }
    let response = request.send().context("failed to call LLM API")?;
    let status = response.status();
    let body = response.text().context("failed to read LLM response")?;
    if !status.is_success() {
        bail!(
            "LLM API HTTP {status}: {}",
            body.chars().take(2000).collect::<String>()
        );
    }
    let value: Value = serde_json::from_str(&body).context("LLM returned invalid JSON")?;
    parse_chat_response(&value)
}

fn parse_chat_response(value: &Value) -> Result<ChatResponse> {
    let message = value
        .pointer("/choices/0/message")
        .context("LLM response has no choices[0].message")?;
    let content = message
        .get("content")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let reasoning_content = message
        .get("reasoning_content")
        .or_else(|| message.get("reasoning"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let finish_reason = value
        .pointer("/choices/0/finish_reason")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let prompt_tokens = value
        .pointer("/usage/prompt_tokens")
        .and_then(Value::as_u64);
    let completion_tokens = value
        .pointer("/usage/completion_tokens")
        .and_then(Value::as_u64);

    Ok(ChatResponse {
        content,
        reasoning_content,
        finish_reason,
        prompt_tokens,
        completion_tokens,
    })
}

fn apply_reasoning(payload: &mut Value, transport: ReasoningTransport, effort: ReasoningEffort) {
    let Some(object) = payload.as_object_mut() else {
        return;
    };
    let Some(value) = effort.api_value() else {
        if matches!(transport, ReasoningTransport::Qwen) {
            object.insert(
                "chat_template_kwargs".into(),
                json!({"enable_thinking": false}),
            );
        }
        return;
    };
    match transport {
        ReasoningTransport::None => {}
        ReasoningTransport::OpenAi => {
            object.insert("reasoning_effort".into(), json!(value));
        }
        ReasoningTransport::Qwen => {
            object.insert(
                "chat_template_kwargs".into(),
                json!({"enable_thinking": true, "reasoning_effort": value}),
            );
        }
        ReasoningTransport::Thinking => {
            object.insert(
                "thinking".into(),
                json!({"type": "enabled", "effort": value}),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qwen_reasoning_is_embedded_per_request() {
        let mut value = json!({});
        apply_reasoning(&mut value, ReasoningTransport::Qwen, ReasoningEffort::Xhigh);
        assert_eq!(value["chat_template_kwargs"]["reasoning_effort"], "xhigh");
    }

    #[test]
    fn detects_reasoning_exhaustion_without_final_content() {
        let value = json!({
            "choices": [{
                "finish_reason": "length",
                "message": {
                    "content": "",
                    "reasoning_content": "still thinking"
                }
            }],
            "usage": {"prompt_tokens": 100, "completion_tokens": 12000}
        });
        let response = parse_chat_response(&value).expect("valid response");
        assert!(response.final_text().is_none());
        assert!(response.exhausted_before_final());
        assert!(response.diagnostics().contains("completion_tokens=12000"));
    }

    #[test]
    fn parses_normal_final_content() {
        let value = json!({
            "choices": [{
                "finish_reason": "stop",
                "message": {
                    "content": "OK",
                    "reasoning_content": "brief thought"
                }
            }]
        });
        let response = parse_chat_response(&value).expect("valid response");
        assert_eq!(response.final_text(), Some("OK"));
        assert!(!response.exhausted_before_final());
    }
}
