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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_profile: Option<String>,
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
            model_profile: None,
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
pub struct ModelProfile {
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub reasoning_transport: ReasoningTransport,
    pub temperature: f32,
    pub timeout_secs: u64,
    pub max_input_tokens: usize,
    pub max_output_tokens: u32,
    pub send_temperature: bool,
    pub completion_token_parameter: String,
    pub input_usd_per_million: Option<f64>,
    pub output_usd_per_million: Option<f64>,
}
impl Default for ModelProfile {
    fn default() -> Self {
        Self {
            name: "New profile".to_owned(),
            base_url: "http://127.0.0.1:8000/v1".to_owned(),
            api_key: String::new(),
            model: "qwen3.8-27b".to_owned(),
            reasoning_transport: ReasoningTransport::OpenAi,
            temperature: 0.2,
            timeout_secs: 900,
            max_input_tokens: 220_000,
            max_output_tokens: 16_384,
            send_temperature: true,
            completion_token_parameter: "max_tokens".to_owned(),
            input_usd_per_million: None,
            output_usd_per_million: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub profiles: Vec<ModelProfile>,
    pub active_profile: Option<String>,
    pub discovery_profile: Option<String>,
    pub map_profile: Option<String>,
    pub max_concurrent_jobs: usize,
    pub max_concurrent_requests: usize,
    pub cache_max_mib: u64,
    pub cache_ttl_days: u64,
    pub model_input_limit: usize,
    pub model_output_limit: u32,
    pub send_temperature: bool,
    pub completion_token_parameter: String,
    pub input_usd_per_million: Option<f64>,
    pub output_usd_per_million: Option<f64>,
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
            profiles: Vec::new(),
            active_profile: None,
            discovery_profile: None,
            map_profile: None,
            max_concurrent_jobs: 2,
            max_concurrent_requests: 4,
            cache_max_mib: 256,
            cache_ttl_days: 30,
            model_input_limit: 220_000,
            model_output_limit: 16_384,
            send_temperature: true,
            completion_token_parameter: "max_tokens".to_owned(),
            input_usd_per_million: None,
            output_usd_per_million: None,
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

impl AppConfig {
    pub fn with_profile(&self, name: Option<&str>) -> Result<Self> {
        let mut config = self.clone();
        if let Some(name) = name.filter(|name| !name.trim().is_empty()) {
            let profile = self
                .profiles
                .iter()
                .find(|profile| profile.name == name)
                .with_context(|| format!("unknown model profile: {name}"))?;
            config.base_url = profile.base_url.clone();
            config.api_key = profile.api_key.clone();
            config.model = profile.model.clone();
            config.reasoning_transport = profile.reasoning_transport;
            config.temperature = profile.temperature;
            config.timeout_secs = profile.timeout_secs;
            config.model_input_limit = profile.max_input_tokens;
            config.model_output_limit = profile.max_output_tokens;
            config.send_temperature = profile.send_temperature;
            config.completion_token_parameter = profile.completion_token_parameter.clone();
            config.input_usd_per_million = profile.input_usd_per_million;
            config.output_usd_per_million = profile.output_usd_per_million;
        }
        Ok(config)
    }
    pub fn update_api_fields(&mut self, name: Option<&str>, api: &Self) -> Result<()> {
        if let Some(name) = name {
            let profile = self
                .profiles
                .iter_mut()
                .find(|profile| profile.name == name)
                .with_context(|| format!("unknown model profile: {name}"))?;
            profile.base_url.clone_from(&api.base_url);
            profile.api_key.clone_from(&api.api_key);
            profile.model.clone_from(&api.model);
            profile.reasoning_transport = api.reasoning_transport;
        } else {
            self.base_url.clone_from(&api.base_url);
            self.api_key.clone_from(&api.api_key);
            self.model.clone_from(&api.model);
            self.reasoning_transport = api.reasoning_transport;
        }
        Ok(())
    }
    pub fn for_tool(&self, name: &str) -> Result<Self> {
        let tool = match name {
            "analyze" => &self.tools.analyze,
            "debug_issue" => &self.tools.debug_issue,
            "plan" => &self.tools.plan,
            "review_diff" => &self.tools.review_diff,
            "document_repo" => &self.tools.document_repo,
            "update_docs" => &self.tools.update_docs,
            _ => anyhow::bail!("unknown tool: {name}"),
        };
        self.with_profile(
            tool.model_profile
                .as_deref()
                .or(self.active_profile.as_deref()),
        )
    }
    pub fn without_credentials(&self) -> Self {
        let mut config = self.clone();
        config.api_key.clear();
        config.system_prompt_prefix = crate::privacy::redact(&config.system_prompt_prefix);
        for profile in &mut config.profiles {
            profile.api_key.clear();
        }
        config
    }
    pub fn restore_credentials(&mut self, current: &Self) {
        self.api_key = if self.base_url == current.base_url {
            current.api_key.clone()
        } else {
            String::new()
        };
        for profile in &mut self.profiles {
            profile.api_key = current
                .profiles
                .iter()
                .find(|live| live.name == profile.name && live.base_url == profile.base_url)
                .map(|live| live.api_key.clone())
                .unwrap_or_default();
        }
    }
    pub fn validate(&self) -> Result<()> {
        let mut names = std::collections::HashSet::new();
        for profile in &self.profiles {
            if profile.name.trim().is_empty() || !names.insert(profile.name.as_str()) {
                anyhow::bail!("profile names must be non-empty and unique");
            }
        }
        for name in [
            self.active_profile.as_deref(),
            self.discovery_profile.as_deref(),
            self.map_profile.as_deref(),
            self.tools.analyze.model_profile.as_deref(),
            self.tools.debug_issue.model_profile.as_deref(),
            self.tools.plan.model_profile.as_deref(),
            self.tools.review_diff.model_profile.as_deref(),
            self.tools.document_repo.model_profile.as_deref(),
            self.tools.update_docs.model_profile.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            self.with_profile(Some(name))?;
        }
        Ok(())
    }
}
pub fn data_dir() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("LLM2MCP_DATA_DIR").filter(|path| !path.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    Ok(ProjectDirs::from("ai", "LLM2MCP", "LLM2MCP")
        .context("cannot resolve data directory")?
        .data_local_dir()
        .to_owned())
}

pub fn config_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("LLM2MCP_CONFIG_DIR").filter(|path| !path.is_empty()) {
        return Ok(PathBuf::from(path).join("config.json"));
    }
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
    config.validate()?;
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
    fn profiles_preserve_old_defaults_and_do_not_restore_keys_to_changed_endpoints() {
        let mut config: AppConfig =
            serde_json::from_value(serde_json::json!({"api_key":"top-secret"})).unwrap();
        assert!(config.profiles.is_empty());
        assert_eq!(config.max_concurrent_jobs, 2);
        config.profiles.push(ModelProfile {
            name: "analysis".to_owned(),
            api_key: "profile-secret".to_owned(),
            model: "analysis-model".to_owned(),
            ..ModelProfile::default()
        });
        config.tools.analyze.model_profile = Some("analysis".to_owned());
        assert_eq!(config.for_tool("analyze").unwrap().model, "analysis-model");
        let mut snapshot = config.without_credentials();
        assert!(snapshot.api_key.is_empty());
        assert!(snapshot.profiles[0].api_key.is_empty());
        snapshot.restore_credentials(&config);
        assert_eq!(snapshot.profiles[0].api_key, "profile-secret");
        snapshot = config.without_credentials();
        snapshot.profiles[0].base_url = "https://other.example/v1".to_owned();
        snapshot.restore_credentials(&config);
        assert!(snapshot.profiles[0].api_key.is_empty());
        config.profiles.push(config.profiles[0].clone());
        assert!(config.validate().is_err());
    }

    #[test]
    fn selected_api_edits_do_not_change_fallback_or_another_profile() {
        let mut config = AppConfig::default();
        config.profiles.push(ModelProfile {
            name: "selected".into(),
            ..Default::default()
        });
        config.profiles.push(ModelProfile {
            name: "other".into(),
            ..Default::default()
        });
        let fallback = config.model.clone();
        let other = config.profiles[1].model.clone();
        let mut edit = config.with_profile(Some("selected")).unwrap();
        edit.model = "refreshed-model".into();
        config.update_api_fields(Some("selected"), &edit).unwrap();
        assert_eq!(
            config.with_profile(Some("selected")).unwrap().model,
            "refreshed-model"
        );
        assert_eq!(config.model, fallback);
        assert_eq!(config.profiles[1].model, other);
        assert!(
            config
                .update_api_fields(Some("deleted-profile"), &edit)
                .is_err()
        );
    }

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
