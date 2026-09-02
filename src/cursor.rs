use anyhow::{Context, Result};
use directories::BaseDirs;
use serde_json::{Map, Value, json};
use std::{fs, path::PathBuf};

pub fn cursor_config_path() -> Result<PathBuf> {
    let base = BaseDirs::new().context("cannot resolve home directory")?;
    Ok(base.home_dir().join(".cursor").join("mcp.json"))
}

pub fn is_installed() -> Result<bool> {
    let path = cursor_config_path()?;
    if !path.exists() {
        return Ok(false);
    }
    let data = fs::read_to_string(path)?;
    let value: Value = serde_json::from_str(&data)?;
    Ok(value.pointer("/mcpServers/llm2mcp").is_some())
}

pub fn install() -> Result<PathBuf> {
    let path = cursor_config_path()?;
    let mut root = if path.exists() {
        serde_json::from_str::<Value>(&fs::read_to_string(&path)?)
            .with_context(|| format!("invalid JSON in {}", path.display()))?
    } else {
        json!({})
    };
    let root_obj = root
        .as_object_mut()
        .context("Cursor mcp.json root must be an object")?;
    let servers = root_obj
        .entry("mcpServers")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .context("mcpServers must be an object")?;

    let exe = std::env::current_exe().context("cannot resolve current executable")?;
    servers.insert(
        "llm2mcp".to_owned(),
        json!({
            "type": "stdio",
            "command": exe.to_string_lossy(),
            "args": ["mcp", "--workspace", "${workspaceFolder}"]
        }),
    );

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, serde_json::to_string_pretty(&root)?)?;
    Ok(path)
}

pub fn remove() -> Result<PathBuf> {
    let path = cursor_config_path()?;
    if !path.exists() {
        return Ok(path);
    }
    let mut root: Value = serde_json::from_str(&fs::read_to_string(&path)?)?;
    if let Some(servers) = root.get_mut("mcpServers").and_then(Value::as_object_mut) {
        servers.remove("llm2mcp");
    }
    fs::write(&path, serde_json::to_string_pretty(&root)?)?;
    Ok(path)
}
