use anyhow::{Context, Result, anyhow, bail};
use directories::BaseDirs;
use serde_json::{Map, Value, json};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use crate::{cursor, install, safe_fs};
use toml_edit::{DocumentMut, value};

const SERVER_NAME: &str = "llm2mcp";
const CLAUDE_TOOL_TIMEOUT_MS: u64 = 1_800_000;
const CODEX_TOOL_TIMEOUT_SEC: f64 = 1_800.0;
const GROK_TOOL_TIMEOUT_SEC: f64 = 6_000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientKind {
    Cursor,
    ClaudeCode,
    Codex,
    GrokBuild,
    Pi,
}

impl ClientKind {
    pub const ALL: [Self; 5] = [
        Self::Cursor,
        Self::ClaudeCode,
        Self::Codex,
        Self::GrokBuild,
        Self::Pi,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Cursor => "Cursor",
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex",
            Self::GrokBuild => "Grok Build",
            Self::Pi => "Pi",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ClientStatus {
    pub kind: ClientKind,
    pub available: bool,
    pub installed: bool,
    pub detail: String,
}

pub fn statuses() -> Vec<ClientStatus> {
    ClientKind::ALL.into_iter().map(status).collect()
}

pub fn status(kind: ClientKind) -> ClientStatus {
    match kind {
        ClientKind::Cursor => ClientStatus {
            kind,
            available: true,
            installed: cursor::is_installed().unwrap_or(false),
            detail: cursor::cursor_config_path()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
        },
        ClientKind::ClaudeCode => cli_status(kind, "claude", &["mcp", "get", SERVER_NAME]),
        ClientKind::Codex => cli_status(kind, "codex", &["mcp", "get", SERVER_NAME]),
        ClientKind::GrokBuild => cli_status(kind, "grok", &["mcp", "list", "--json"]),
        ClientKind::Pi => ClientStatus {
            kind,
            available: find_executable("pi").is_some(),
            installed: pi_is_installed().unwrap_or(false),
            detail: pi_config_path()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
        },
    }
}

fn cli_status(kind: ClientKind, command: &str, args: &[&str]) -> ClientStatus {
    let Some(path) = find_executable(command) else {
        return ClientStatus {
            kind,
            available: false,
            installed: false,
            detail: String::new(),
        };
    };

    let installed = Command::new(&path)
        .args(args)
        .output()
        .map(|output| {
            if kind == ClientKind::GrokBuild {
                output.status.success()
                    && String::from_utf8_lossy(&output.stdout).contains(SERVER_NAME)
            } else {
                output.status.success()
            }
        })
        .unwrap_or(false);

    ClientStatus {
        kind,
        available: true,
        installed,
        detail: path.display().to_string(),
    }
}

pub fn install(kind: ClientKind) -> Result<String> {
    let stable_exe = install::ensure_stable_install()?;
    let exe = stable_exe.to_string_lossy().into_owned();
    match kind {
        ClientKind::Cursor => cursor::install(&stable_exe)
            .map(|path| format!("{}; binary={}", path.display(), stable_exe.display())),
        ClientKind::ClaudeCode => {
            run_cli(
                "claude",
                &[
                    "mcp",
                    "add",
                    SERVER_NAME,
                    "--scope",
                    "user",
                    "--",
                    &exe,
                    "mcp",
                ],
            )?;
            let note = configure_claude_timeout()
                .map(|_| "tool timeout=30m".to_owned())
                .unwrap_or_else(|error| format!("timeout patch skipped: {error}"));
            Ok(format!(
                "Claude Code user MCP; binary={}; {note}",
                stable_exe.display()
            ))
        }
        ClientKind::Codex => {
            run_cli("codex", &["mcp", "add", SERVER_NAME, "--", &exe, "mcp"])?;
            let note = configure_toml_timeout(&codex_config_path()?, CODEX_TOOL_TIMEOUT_SEC)
                .map(|_| "tool_timeout_sec=1800".to_owned())
                .unwrap_or_else(|error| format!("timeout patch skipped: {error}"));
            Ok(format!(
                "Codex user MCP; binary={}; {note}",
                stable_exe.display()
            ))
        }
        ClientKind::GrokBuild => {
            run_cli("grok", &["mcp", "add", SERVER_NAME, "--", &exe, "mcp"])?;
            let note = configure_toml_timeout(&grok_config_path()?, GROK_TOOL_TIMEOUT_SEC)
                .map(|_| "tool_timeout_sec=6000".to_owned())
                .unwrap_or_else(|error| format!("timeout patch skipped: {error}"));
            Ok(format!(
                "Grok Build user MCP; binary={}; {note}",
                stable_exe.display()
            ))
        }
        ClientKind::Pi => install_pi(&stable_exe),
    }
}

pub fn remove(kind: ClientKind) -> Result<String> {
    match kind {
        ClientKind::Cursor => cursor::remove().map(|path| path.display().to_string()),
        ClientKind::ClaudeCode => {
            run_cli("claude", &["mcp", "remove", SERVER_NAME, "--scope", "user"])?;
            Ok("Claude Code user MCP".to_owned())
        }
        ClientKind::Codex => {
            run_cli("codex", &["mcp", "remove", SERVER_NAME])?;
            Ok("Codex user MCP".to_owned())
        }
        ClientKind::GrokBuild => {
            run_cli("grok", &["mcp", "remove", SERVER_NAME])?;
            Ok("Grok Build user MCP".to_owned())
        }
        ClientKind::Pi => remove_pi(),
    }
}

fn run_cli(command: &str, args: &[&str]) -> Result<Output> {
    let executable = find_executable(command)
        .ok_or_else(|| anyhow!("{command} executable was not found in PATH"))?;
    let output = Command::new(executable)
        .args(args)
        .output()
        .with_context(|| format!("failed to run {command}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        bail!("{command} failed: {}{}", stdout.trim(), stderr.trim());
    }
    Ok(output)
}

fn pi_config_path() -> Result<PathBuf> {
    let base = BaseDirs::new().context("cannot resolve home directory")?;
    Ok(base.home_dir().join(".pi").join("agent").join("mcp.json"))
}

fn pi_is_installed() -> Result<bool> {
    let path = pi_config_path()?;
    if !path.exists() {
        return Ok(false);
    }
    let value: Value = serde_json::from_str(&fs::read_to_string(path)?)?;
    Ok(value.pointer("/mcpServers/llm2mcp").is_some())
}

fn install_pi(executable: &Path) -> Result<String> {
    if find_executable("pi").is_none() {
        bail!("pi executable was not found in PATH");
    }

    // Pi currently relies on an MCP extension rather than shipping MCP as a
    // core feature. Re-running this command is safe for an existing package.
    let _ = run_cli("pi", &["install", "npm:pi-mcp-extension"])?;

    let path = pi_config_path()?;
    merge_json_server(&path, false, executable)?;
    Ok(path.display().to_string())
}

fn remove_pi() -> Result<String> {
    let path = pi_config_path()?;
    if !path.exists() {
        return Ok(path.display().to_string());
    }
    let mut root: Value = serde_json::from_str(&fs::read_to_string(&path)?)?;
    if let Some(servers) = root.get_mut("mcpServers").and_then(Value::as_object_mut) {
        servers.remove(SERVER_NAME);
    }
    safe_fs::atomic_write_with_backup(&path, serde_json::to_string_pretty(&root)?.as_bytes())?;
    Ok(path.display().to_string())
}

fn merge_json_server(path: &Path, include_type: bool, executable: &Path) -> Result<()> {
    let mut root = if path.exists() {
        serde_json::from_str::<Value>(&fs::read_to_string(path)?)
            .with_context(|| format!("invalid JSON in {}", path.display()))?
    } else {
        json!({})
    };
    let object = root
        .as_object_mut()
        .context("MCP config root must be an object")?;
    let servers = object
        .entry("mcpServers")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .context("mcpServers must be an object")?;

    let mut entry = json!({
        "command": executable.to_string_lossy(),
        "args": ["mcp"]
    });
    if include_type {
        entry["type"] = json!("stdio");
    }
    servers.insert(SERVER_NAME.to_owned(), entry);

    safe_fs::atomic_write_with_backup(path, serde_json::to_string_pretty(&root)?.as_bytes())?;
    Ok(())
}

fn claude_config_path() -> Result<PathBuf> {
    let base = BaseDirs::new().context("cannot resolve home directory")?;
    Ok(base.home_dir().join(".claude.json"))
}

fn codex_config_path() -> Result<PathBuf> {
    if let Some(home) = env::var_os("CODEX_HOME") {
        return Ok(PathBuf::from(home).join("config.toml"));
    }
    let base = BaseDirs::new().context("cannot resolve home directory")?;
    Ok(base.home_dir().join(".codex").join("config.toml"))
}

fn grok_config_path() -> Result<PathBuf> {
    if let Some(home) = env::var_os("GROK_HOME") {
        return Ok(PathBuf::from(home).join("config.toml"));
    }
    let base = BaseDirs::new().context("cannot resolve home directory")?;
    Ok(base.home_dir().join(".grok").join("config.toml"))
}

fn configure_claude_timeout() -> Result<()> {
    let path = claude_config_path()?;
    let mut root: Value = serde_json::from_str(
        &fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?,
    )?;
    let server = root
        .get_mut("mcpServers")
        .and_then(Value::as_object_mut)
        .and_then(|servers| servers.get_mut(SERVER_NAME))
        .and_then(Value::as_object_mut)
        .context("Claude Code user MCP entry was not found after install")?;
    server.insert("timeout".to_owned(), json!(CLAUDE_TOOL_TIMEOUT_MS));
    server.insert(
        "request_timeout_ms".to_owned(),
        json!(CLAUDE_TOOL_TIMEOUT_MS),
    );
    safe_fs::atomic_write_with_backup(&path, serde_json::to_string_pretty(&root)?.as_bytes())?;
    Ok(())
}

fn configure_toml_timeout(path: &Path, seconds: f64) -> Result<()> {
    let source =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut document = source
        .parse::<DocumentMut>()
        .with_context(|| format!("invalid TOML in {}", path.display()))?;
    if !document["mcp_servers"][SERVER_NAME].is_table_like() {
        bail!(
            "MCP entry {SERVER_NAME} was not found in {}",
            path.display()
        );
    }
    document["mcp_servers"][SERVER_NAME]["tool_timeout_sec"] = value(seconds);
    safe_fs::atomic_write_with_backup(path, document.to_string().as_bytes())?;
    Ok(())
}

fn find_executable(name: &str) -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    let candidates: Vec<String> = if cfg!(windows) {
        let extensions = env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".to_owned());
        extensions
            .split(';')
            .map(|ext| format!("{name}{ext}"))
            .chain(std::iter::once(name.to_owned()))
            .collect()
    } else {
        vec![name.to_owned()]
    };

    for directory in env::split_paths(&path_var) {
        for candidate in &candidates {
            let path = directory.join(candidate);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::ClientKind;

    #[test]
    fn client_names_are_stable() {
        assert_eq!(ClientKind::ALL.len(), 5);
        assert_eq!(ClientKind::Codex.label(), "Codex");
        assert_eq!(ClientKind::GrokBuild.label(), "Grok Build");
    }
}
