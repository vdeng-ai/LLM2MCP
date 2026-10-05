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
            .filter(|check| {
                check.name == "LLM inference"
                    || check.name == "Local stdio MCP"
                    || check.name == "Deep MCP lifecycle"
            })
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

struct Session {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    replies: std::sync::mpsc::Receiver<serde_json::Value>,
    sequence: u64,
}
impl Session {
    fn new(config: &AppConfig, workspace: &std::path::Path) -> Result<Self> {
        use std::io::BufRead;
        let mut command = Command::new(std::env::current_exe()?);
        command
            .args(["mcp", "--workspace"])
            .arg(workspace)
            .env("LLM2MCP_ANALYZE_EXECUTION", "async")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        if let Some(profile) = &config.active_profile {
            command.env("LLM2MCP_ACTIVE_PROFILE", profile);
        }
        let mut child = command.spawn()?;
        let stdin = child.stdin.take().context("missing MCP stdin")?;
        let stdout = child.stdout.take().context("missing MCP stdout")?;
        let (sender, replies) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout).lines() {
                let Ok(line) = line else {
                    break;
                };
                if let Ok(reply) = serde_json::from_str(&line) {
                    let _ = sender.send(reply);
                }
            }
        });
        Ok(Self {
            child,
            stdin,
            replies,
            sequence: 0,
        })
    }
    fn call(&mut self, method: &str, mut params: serde_json::Value) -> Result<serde_json::Value> {
        use std::io::Write;
        self.sequence += 1;
        params["_meta"] = serde_json::json!({
            "io.modelcontextprotocol/protocolVersion":"2026-07-28",
            "io.modelcontextprotocol/clientCapabilities":{"extensions":{"io.modelcontextprotocol/tasks":{}}}
        });
        writeln!(
            self.stdin,
            "{}",
            serde_json::json!({"jsonrpc":"2.0","id":self.sequence,"method":method,"params":params})
        )?;
        self.stdin.flush()?;
        let started = std::time::Instant::now();
        loop {
            Control::current().check()?;
            if started.elapsed() > Duration::from_secs(30) {
                bail!("MCP diagnostic RPC timeout");
            }
            match self.replies.recv_timeout(Duration::from_millis(100)) {
                Ok(reply) if reply["id"] == self.sequence => {
                    if reply.get("error").is_some() {
                        bail!("MCP diagnostic RPC failed: {}", reply["error"]);
                    }
                    return Ok(reply["result"].clone());
                }
                Ok(_) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => bail!("MCP diagnostic process exited"),
            }
        }
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
struct ProbeJob(String, bool);
impl Drop for ProbeJob {
    fn drop(&mut self) {
        let _ = crate::jobs::cancel(&self.0);
        if self.1 {
            let _ = crate::jobs::mark_cancelled(&self.0);
        }
    }
}

fn deep_probe(config: &AppConfig, workspace: &std::path::Path, path: &str) -> Result<String> {
    let workspace = workspace.canonicalize()?;
    // Validate that this explicit diagnostic path is readable under normal workspace rules.
    let sources =
        crate::workspace::collect_filtered(&workspace, &[path.to_owned()], config, &[], &[])?;
    if sources.included_files.len() != 1 {
        bail!("--path must identify one readable source file");
    }
    let mut session = Session::new(config, &workspace)?;
    let discovery = session.call("server/discover", serde_json::json!({}))?;
    if discovery
        .pointer("/capabilities/extensions/io.modelcontextprotocol~1tasks")
        .is_none()
    {
        bail!("Tasks extension missing");
    }
    let started = session.call("tools/call", serde_json::json!({"name":"analyze","arguments":{"task":"Describe this diagnostic source briefly, grounded in evidence.","paths":[path]}}))?;
    let id = started["taskId"]
        .as_str()
        .context("analysis did not return a durable task")?
        .to_owned();
    let _guard = ProbeJob(id.clone(), false);
    drop(session);
    let mut reconnected = Session::new(config, &workspace)?;
    let deadline = std::time::Instant::now()
        + Duration::from_secs(config.timeout_secs.saturating_mul(3).clamp(30, 600));
    let result = loop {
        Control::current().check()?;
        let status = reconnected.call("tasks/get", serde_json::json!({"taskId":id}))?;
        match status["status"].as_str() {
            Some("completed") => break status["result"].clone(),
            Some("failed" | "cancelled") => bail!("diagnostic analysis failed: {}", status),
            _ => {}
        }
        if std::time::Instant::now() > deadline {
            bail!("diagnostic task timeout; cancellation requested");
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    if result["isError"] == true {
        bail!("diagnostic tool result is an error");
    }
    let mut portable = reconnected.call(
        "tools/call",
        serde_json::json!({"name":"job_result","arguments":{"job_id":id}}),
    )?;
    // Per-request server metadata is transport information, not the stored tool payload.
    if let Some(object) = portable.as_object_mut()
        && let Some(meta) = object
            .get_mut("_meta")
            .and_then(serde_json::Value::as_object_mut)
    {
        meta.remove("io.modelcontextprotocol/serverInfo");
        if meta.is_empty() {
            object.remove("_meta");
        }
    }
    if portable != result {
        bail!("portable result differs from Tasks result");
    }
    // A synthetic queued handle tests cancellation deterministically without another inference call.
    let queued = crate::jobs::create_sync(
        "analyze",
        &workspace,
        serde_json::json!({"task":"doctor cancellation probe","paths":[path]}),
        1,
        1000,
    )?;
    let _cancel_guard = ProbeJob(queued.id.clone(), true);
    reconnected.call(
        "tasks/update",
        serde_json::json!({"taskId":queued.id,"inputResponses":{}}),
    )?;
    reconnected.call("tasks/cancel", serde_json::json!({"taskId":queued.id}))?;
    if !crate::jobs::load(&queued.id)?.cancel_requested {
        bail!("queued cancellation intent was not persisted");
    }
    // No worker runs this synthetic job; acknowledge the cooperative request locally.
    crate::jobs::mark_cancelled(&queued.id)?;
    let cancelled = reconnected.call("tasks/get", serde_json::json!({"taskId":queued.id}))?;
    if cancelled["status"] != "cancelled" {
        bail!("queued cancellation did not reach cancelled state");
    }
    Ok(format!(
        "real analyze tools/call -> durable Tasks -> reconnect -> completed result == portable job_result; queued cancellation passed; task={id}. Host application execution remains a separate check."
    ))
}

pub fn deep_check(config: &AppConfig, workspace: &std::path::Path, path: &str) -> Check {
    let result = deep_probe(config, workspace, path);
    Check {
        name: "Deep MCP lifecycle".into(),
        ok: result.is_ok(),
        detail: crate::privacy::redact(&result.unwrap_or_else(|error| format!("{error:#}"))),
    }
}
