use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<ExecutionMode>,
}

impl ToolConfig {
    pub fn new(
        reasoning: ReasoningEffort,
        max_output_tokens: u32,
        execution: ExecutionMode,
    ) -> Self {
        Self {
            reasoning,
            max_output_tokens,
            execution: Some(execution),
        }
    }

    pub fn execution_or(&self, fallback: ExecutionMode) -> ExecutionMode {
        self.execution.unwrap_or(fallback)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolsConfig {
    pub analyze: ToolConfig,
    pub plan: ToolConfig,
    pub review_diff: ToolConfig,
    pub document_repo: ToolConfig,
    pub update_docs: ToolConfig,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            analyze: ToolConfig::new(ReasoningEffort::Medium, 3000, ExecutionMode::Auto),
            plan: ToolConfig::new(ReasoningEffort::Xhigh, 4000, ExecutionMode::Auto),
            review_diff: ToolConfig::new(ReasoningEffort::Medium, 3000, ExecutionMode::Sync),
            document_repo: ToolConfig::new(ReasoningEffort::Medium, 12_000, ExecutionMode::Async),
            update_docs: ToolConfig::new(ReasoningEffort::Xhigh, 8_000, ExecutionMode::Async),
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
    pub temperature: f32,
    pub timeout_secs: u64,
    pub max_source_chars: usize,
    pub max_file_chars: usize,
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
            temperature: 0.2,
            timeout_secs: 900,
            max_source_chars: 220_000,
            max_file_chars: 60_000,
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
        serde_json::from_str(&data)
            .with_context(|| format!("failed to parse {}", path.display()))?
    } else {
        AppConfig::default()
    };
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
    fs::write(&path, data).with_context(|| format!("failed to write {}", path.display()))?;

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
        assert_eq!(
            config.tools.document_repo.reasoning,
            ReasoningEffort::Medium
        );
        assert_eq!(config.document_map_concurrency, 2);
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
