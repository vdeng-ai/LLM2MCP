use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

use crate::safe_fs;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    #[default]
    English,
    SimplifiedChinese,
}

impl Language {
    pub const ALL: [Self; 2] = [Self::English, Self::SimplifiedChinese];

    pub fn label(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::SimplifiedChinese => "简体中文",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningTransport {
    None,
    #[default]
    OpenAi,
    Qwen,
    Thinking,
}

impl ReasoningTransport {
    pub const ALL: [Self; 4] = [Self::None, Self::OpenAi, Self::Qwen, Self::Thinking];

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::OpenAi => "OpenAI reasoning_effort",
            Self::Qwen => "Qwen chat_template_kwargs",
            Self::Thinking => "thinking parameter",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    Off,
    Low,
    #[default]
    Medium,
    Xhigh,
}

impl ReasoningEffort {
    pub const ALL: [Self; 4] = [Self::Off, Self::Low, Self::Medium, Self::Xhigh];

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::Xhigh => "XHigh",
        }
    }

    pub fn api_value(self) -> Option<&'static str> {
        match self {
            Self::Off => None,
            Self::Low => Some("low"),
            Self::Medium => Some("medium"),
            Self::Xhigh => Some("xhigh"),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    Sync,
    #[default]
    Auto,
    Async,
}

impl ExecutionMode {
    pub const ALL: [Self; 3] = [Self::Sync, Self::Auto, Self::Async];
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolConfig {
    pub reasoning: ReasoningEffort,
    pub max_output_tokens: u32,
    /// Maximum tokens returned to the primary coding agent after local compaction.
    /// None means the tool returns its full generated artifact (used for docs tools).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_return_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<ExecutionMode>,
}

impl ToolConfig {
    pub fn new(
        reasoning: ReasoningEffort,
        max_output_tokens: u32,
        primary_return_tokens: Option<u32>,
        execution: ExecutionMode,
    ) -> Self {
        Self {
            reasoning,
            max_output_tokens,
            primary_return_tokens,
            execution: Some(execution),
        }
    }

    pub fn execution_or(&self, fallback: ExecutionMode) -> ExecutionMode {
        self.execution.unwrap_or(fallback)
    }

    pub fn primary_return_or(&self, fallback: u32) -> u32 {
        self.primary_return_tokens.unwrap_or(fallback).max(128)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolsConfig {
    pub analyze: ToolConfig,
    pub debug_issue: ToolConfig,
    pub plan: ToolConfig,
    pub review_diff: ToolConfig,
    pub document_repo: ToolConfig,
    pub update_docs: ToolConfig,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            analyze: ToolConfig::new(
                ReasoningEffort::Medium,
                4_000,
                Some(800),
                ExecutionMode::Auto,
            ),
            debug_issue: ToolConfig::new(
                ReasoningEffort::Xhigh,
                6_000,
                Some(1_400),
                ExecutionMode::Auto,
            ),
            plan: ToolConfig::new(
                ReasoningEffort::Xhigh,
                6_000,
                Some(1_200),
                ExecutionMode::Auto,
            ),
            review_diff: ToolConfig::new(
                ReasoningEffort::Medium,
                4_000,
                Some(1_000),
                ExecutionMode::Sync,
            ),
            document_repo: ToolConfig::new(
                ReasoningEffort::Medium,
                12_000,
                None,
                ExecutionMode::Async,
            ),
            update_docs: ToolConfig::new(ReasoningEffort::Xhigh, 8_000, None, ExecutionMode::Async),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub language: Language,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub reasoning_transport: ReasoningTransport,
    pub system_prompt_prefix: String,
    pub temperature: f32,
    pub timeout_secs: u64,
    pub job_ttl_hours: u64,
    pub job_poll_interval_ms: u64,
    /// Legacy character limits kept for config-file migration only. New source
    /// collection uses the token budgets below and these fields are not re-saved.
    #[serde(skip_serializing)]
    pub max_source_chars: usize,
    #[serde(skip_serializing)]
    pub max_file_chars: usize,
    pub max_source_tokens: usize,
    pub max_file_tokens: usize,
    pub discovery_index_tokens: usize,
    pub document_map_concurrency: usize,
    pub tools: ToolsConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            language: Language::English,
            base_url: "http://127.0.0.1:8000/v1".to_owned(),
            api_key: String::new(),
            model: "qwen3.8-27b".to_owned(),
            reasoning_transport: ReasoningTransport::OpenAi,
            system_prompt_prefix: String::new(),
            temperature: 0.2,
            timeout_secs: 900,
            job_ttl_hours: 7 * 24,
            job_poll_interval_ms: 5_000,
            max_source_chars: 220_000,
            max_file_chars: 60_000,
            max_source_tokens: 55_000,
            max_file_tokens: 15_000,
            discovery_index_tokens: 8_000,
            document_map_concurrency: 2,
            tools: ToolsConfig::default(),
        }
    }
}

pub fn config_path() -> Result<PathBuf> {
    let dirs = ProjectDirs::from("ai", "LLM2MCP", "LLM2MCP")
        .context("cannot resolve the user configuration directory")?;
    Ok(dirs.config_dir().join("config.json"))
}

pub fn load() -> Result<AppConfig> {
    let path = config_path()?;
    let mut config = if path.exists() {
        let data = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let raw: serde_json::Value = serde_json::from_str(&data)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        let had_source_tokens = raw.get("max_source_tokens").is_some();
        let had_file_tokens = raw.get("max_file_tokens").is_some();
        let had_analyze_return = raw
            .pointer("/tools/analyze/primary_return_tokens")
            .is_some();
        let had_plan_return = raw.pointer("/tools/plan/primary_return_tokens").is_some();
        let had_review_return = raw
            .pointer("/tools/review_diff/primary_return_tokens")
            .is_some();
        let mut loaded: AppConfig = serde_json::from_value(raw)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        if !had_source_tokens {
            loaded.max_source_tokens = (loaded.max_source_chars / 4).max(2_500);
        }
        if !had_file_tokens {
            loaded.max_file_tokens = (loaded.max_file_chars / 4).max(1_250);
        }
        if !had_analyze_return {
            loaded.tools.analyze.primary_return_tokens = Some(800);
        }
        if !had_plan_return {
            loaded.tools.plan.primary_return_tokens = Some(1_200);
        }
        if !had_review_return {
            loaded.tools.review_diff.primary_return_tokens = Some(1_000);
        }
        loaded
    } else {
        AppConfig::default()
    };
    config.job_ttl_hours = config.job_ttl_hours.clamp(1, 24 * 365);
    config.job_poll_interval_ms = config.job_poll_interval_ms.clamp(1_000, 60_000);
    apply_env_overrides(&mut config);
    Ok(config)
}

fn apply_env_overrides(config: &mut AppConfig) {
    if let Ok(value) = std::env::var("LLM2MCP_BASE_URL")
        && !value.trim().is_empty()
    {
        config.base_url = value;
    }
    if let Ok(value) = std::env::var("LLM2MCP_MODEL")
        && !value.trim().is_empty()
    {
        config.model = value;
    }
    if let Ok(value) = std::env::var("LLM2MCP_API_KEY") {
        config.api_key = value;
    }
}

pub fn save(config: &AppConfig) -> Result<()> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let data = serde_json::to_string_pretty(config)?;
    safe_fs::atomic_write_with_backup(&path, data.as_bytes())
        .with_context(|| format!("failed to write {}", path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("failed to secure {}", path.display()))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_budgets_replace_legacy_character_fields_when_saving() {
        let value = serde_json::to_value(AppConfig::default()).expect("serialize config");
        assert!(value.get("max_source_tokens").is_some());
        assert!(value.get("max_file_tokens").is_some());
        assert!(value.get("max_source_chars").is_none());
        assert!(value.get("max_file_chars").is_none());
    }

    #[test]
    fn job_defaults_are_safe_for_durable_background_work() {
        let config = AppConfig::default();
        assert_eq!(config.job_ttl_hours, 168);
        assert_eq!(config.job_poll_interval_ms, 5_000);
        assert!(config.system_prompt_prefix.is_empty());
    }

    #[test]
    fn old_three_tool_config_gets_new_documentation_defaults() {
        let value = serde_json::json!({
            "tools": {
                "analyze": {"reasoning": "medium", "max_output_tokens": 3000},
                "plan": {"reasoning": "xhigh", "max_output_tokens": 4000},
                "review_diff": {"reasoning": "medium", "max_output_tokens": 3000}
            }
        });
        let config: AppConfig = serde_json::from_value(value).expect("old config should migrate");
        assert_eq!(config.tools.document_repo.max_output_tokens, 12_000);
        assert_eq!(config.tools.debug_issue.max_output_tokens, 6_000);
        assert_eq!(config.tools.debug_issue.reasoning, ReasoningEffort::Xhigh);
        assert_eq!(config.tools.debug_issue.primary_return_tokens, Some(1_400));
        assert_eq!(
            config.tools.document_repo.reasoning,
            ReasoningEffort::Medium
        );
        assert_eq!(config.document_map_concurrency, 2);
        assert_eq!(config.max_source_tokens, 55_000);
        assert_eq!(config.max_file_tokens, 15_000);
        assert_eq!(config.discovery_index_tokens, 8_000);
        assert_eq!(config.tools.update_docs.max_output_tokens, 8_000);
        assert_eq!(
            config
                .tools
                .document_repo
                .execution_or(ExecutionMode::Async),
            ExecutionMode::Async
        );
        assert_eq!(
            config.tools.update_docs.execution_or(ExecutionMode::Async),
            ExecutionMode::Async
        );
    }
}
