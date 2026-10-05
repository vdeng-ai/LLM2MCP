use anyhow::{Context, Result, bail};
use reqwest::{Client, RequestBuilder};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

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
        if matches!(
            self.finish_reason.as_deref(),
            Some("length" | "content_filter")
        ) {
            return None;
        }
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

    #[cfg(test)]
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
    let started = Instant::now();
    let listing = list_models(config);
    if let Ok(models) = &listing
        && !models.is_empty()
        && !models.contains(&config.model)
    {
        bail!("selected model '{}' is absent from /models", config.model);
    }
    let response = chat_detailed(
        config,
        "Connectivity probe: reply with OK.",
        "Reply with OK only.",
        ReasoningEffort::Off,
        128,
    )?;
    if response.final_text().is_none() {
        bail!(
            "inference returned no final content: {}",
            response.diagnostics()
        );
    }
    Ok(format!(
        "Inference OK: {} · {} ms · {}{}",
        config.model,
        started.elapsed().as_millis(),
        response.diagnostics(),
        listing
            .err()
            .map(|error| format!("; /models unavailable: {error:#}"))
            .unwrap_or_default()
    ))
}

fn request_text(request: RequestBuilder, timeout_secs: u64) -> Result<String> {
    let control = crate::control::Control::current();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let fetch = async {
            for attempt in 0..=2 {
                control.check()?;
                let mut response = request.try_clone().context("request cannot be retried")?.send().await.context("failed to connect to LLM API")?;
                let status = response.status();
                let retry_after = response.headers().get("retry-after").and_then(|value| value.to_str().ok()).and_then(|value| value.parse::<u64>().ok()).unwrap_or(1 << attempt).min(10);
                let mut body = Vec::new();
                while let Some(chunk) = response.chunk().await.context("failed to read LLM response")? {
                    if body.len().saturating_add(chunk.len()) > 16 * 1024 * 1024 { bail!("LLM response exceeds 16 MiB limit"); } body.extend_from_slice(&chunk);
                }
                if status.is_success() { return String::from_utf8(body).context("LLM response is not UTF-8"); }
                if attempt < 2 && matches!(status.as_u16(), 429 | 502 | 503 | 504) { tokio::time::sleep(Duration::from_secs(retry_after)).await; continue; }
                bail!("LLM API HTTP {status}: {}", crate::privacy::redact(&String::from_utf8_lossy(&body)).chars().take(2000).collect::<String>());
            }
            unreachable!("retry loop always returns")
        };
        let cancellation = async { loop { control.check()?; tokio::time::sleep(Duration::from_millis(150)).await; } #[allow(unreachable_code)] Ok::<String, anyhow::Error>(String::new()) };
        tokio::select! { result = tokio::time::timeout(Duration::from_secs(timeout_secs.max(1)), fetch) => result.context("LLM request timed out")?, result = cancellation => result }
    })
}

pub fn list_models(config: &AppConfig) -> Result<Vec<String>> {
    let _slot = crate::scheduler::Slot::acquire(
        "http",
        config.max_concurrent_requests,
        &crate::control::Control::current(),
    )?;
    let client = client(config)?;
    let url = format!("{}/models", config.base_url.trim_end_matches('/'));
    let mut request = client.get(url);
    if !config.api_key.trim().is_empty() {
        request = request.bearer_auth(config.api_key.trim());
    }
    let body = request_text(request, config.timeout_secs)?;
    let value: Value = serde_json::from_str(&body).context("LLM /models returned invalid JSON")?;
    let mut models = value
        .get("data")
        .and_then(Value::as_array)
        .context("LLM /models response has no data array")?
        .iter()
        .filter_map(|item| item.get("id").and_then(Value::as_str))
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    models.sort();
    models.dedup();
    Ok(models)
}

fn client(config: &AppConfig) -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(config.timeout_secs.max(5)))
        .build()
        .context("failed to build HTTP client")
}

pub fn chat_detailed(
    config: &AppConfig,
    system: &str,
    user: &str,
    effort: ReasoningEffort,
    max_output_tokens: u32,
) -> Result<ChatResponse> {
    let _slot = crate::scheduler::Slot::acquire(
        "http",
        config.max_concurrent_requests,
        &crate::control::Control::current(),
    )?;
    let client = client(config)?;
    let effective_system = if config.system_prompt_prefix.trim().is_empty() {
        system.to_owned()
    } else {
        format!("{}\n\n{}", config.system_prompt_prefix.trim(), system)
    };
    let mut payload = json!({
        "model": config.model,
        "messages": [
            {"role": "system", "content": crate::privacy::redact(&effective_system)},
            {"role": "user", "content": crate::privacy::redact(user)}
        ],
        "stream": false
    });
    if !["max_tokens", "max_completion_tokens"]
        .contains(&config.completion_token_parameter.as_str())
    {
        bail!("unsupported output token parameter");
    }
    let output_limit = max_output_tokens.min(config.model_output_limit.max(128));
    payload[&config.completion_token_parameter] = json!(output_limit);
    if config.send_temperature {
        payload["temperature"] = json!(config.temperature);
    }
    let input_tokens = crate::workspace::estimate_tokens(&effective_system)
        .saturating_add(crate::workspace::estimate_tokens(user))
        .saturating_add(128);
    if input_tokens.saturating_add(output_limit as usize) > config.model_input_limit {
        bail!(
            "estimated input {input_tokens} plus output {output_limit} exceeds model context limit {}",
            config.model_input_limit
        );
    }
    apply_reasoning(&mut payload, config.reasoning_transport, effort);

    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
    let mut request = client.post(url).json(&payload);
    if !config.api_key.trim().is_empty() {
        request = request.bearer_auth(config.api_key.trim());
    }
    let body = request_text(request, config.timeout_secs)?;
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
