use crate::{config::AppConfig, control::Control};
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::{process::Command, time::Duration};
#[derive(Serialize)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}
#[derive(Serialize)]
pub struct Report {
    pub checks: Vec<Check>,
}
impl Report {
    pub fn ok(&self) -> bool {
        self.checks
            .iter()
            .filter(|check| check.name == "LLM inference" || check.name == "Local stdio MCP")
            .all(|check| check.ok)
    }
    pub fn text(&self) -> String {
        self.checks
            .iter()
            .map(|check| {
                format!(
                    "{} {}: {}",
                    if check.ok { "OK" } else { "WARN" },
                    check.name,
                    check.detail
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}
fn mcp_probe() -> Result<String> {
    let input = concat!(
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-06-18\"}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"ping\"}\n"
    );
    let output = crate::process::run_with_input(
        Command::new(std::env::current_exe()?).arg("mcp"),
        Duration::from_secs(10),
        &Control::current(),
        Some(input.as_bytes()),
    )?;
    if !output.status.success() {
        bail!(
            "MCP probe failed: {}",
            crate::privacy::redact(&String::from_utf8_lossy(&output.stderr))
        );
    }
    let replies = String::from_utf8(output.stdout)?
        .lines()
        .map(serde_json::from_str::<serde_json::Value>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for id in 1..=3 {
        let reply = replies
            .iter()
            .find(|reply| reply["id"] == id)
            .context("missing MCP response")?;
        if reply.get("error").is_some() {
            bail!("MCP RPC error: {}", reply["error"]);
        }
    }
    let tools = replies
        .iter()
        .find(|reply| reply["id"] == 2)
        .and_then(|reply| reply.pointer("/result/tools"))
        .and_then(serde_json::Value::as_array)
        .context("missing tools array")?;
    Ok(format!(
        "initialize / tools/list / ping passed; {} tools",
        tools.len()
    ))
}
pub fn run(config: &AppConfig) -> Report {
    let inference = config
        .with_profile(config.active_profile.as_deref())
        .and_then(|mut config| {
            config.timeout_secs = config.timeout_secs.min(30);
            crate::llm::test_connection(&config)
        });
    let mut checks = Vec::new();
    for (name, result) in [
        ("LLM inference", inference),
        ("Local stdio MCP", mcp_probe()),
    ] {
        checks.push(Check {
            name: name.to_owned(),
            ok: result.is_ok(),
            detail: crate::privacy::redact(&result.unwrap_or_else(|error| format!("{error:#}"))),
        });
    }
    for client in crate::clients::statuses() {
        checks.push(Check { name: client.kind.label().to_owned(), ok: client.available && client.installed, detail: format!("available={} registered={}; {}; host runtime/Tasks requires host-side verification", client.available, client.installed, client.detail) });
    }
    Report { checks }
}
