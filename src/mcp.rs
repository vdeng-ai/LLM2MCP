use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    io::{self, BufRead, Write},
    path::Path,
    thread,
};

use crate::{
    config::{self, AppConfig, ExecutionMode, ReasoningEffort, ToolConfig},
    doc_cache,
    jobs::{self, JobState, Reporter},
    llm, workspace,
};

const LEGACY_PROTOCOL_VERSION: &str = "2025-06-18";
const MODERN_PROTOCOL_VERSION: &str = "2026-07-28";
const TASKS_EXTENSION: &str = "io.modelcontextprotocol/tasks";

const ANALYZE_SYSTEM: &str = r#"You are a local secondary code-analysis model assisting a primary coding agent.
Your job is context offloading: inspect the supplied source material and return only the information the primary agent needs.
Rules:
- Do not reproduce large source blocks.
- Prefer concise, evidence-based conclusions.
- Mention exact file paths and symbols when possible.
- Separate confirmed evidence from inference.
- Identify root causes, risks, and next actions.
- Do not claim you modified files.
- Do not reveal hidden chain-of-thought.
Return a compact engineering report."#;

const PLAN_SYSTEM: &str = r#"You are a local software-planning specialist assisting a primary coding agent.
Analyze the supplied repository information and produce an implementation plan.
Rules:
- Do not repeat the source material.
- Preserve important constraints.
- Mention concrete files, modules, and symbols.
- Prefer the smallest implementation that satisfies the task.
- Identify dependencies, risks, compatibility concerns, and verification steps.
- Do not claim changes have already been made.
- Do not reveal hidden chain-of-thought."#;

const REVIEW_SYSTEM: &str = r#"You are a local code-review specialist assisting a primary coding agent.
Review the supplied git diff for correctness bugs, regressions, concurrency issues, resource leaks, security problems, API compatibility, and missing tests.
Do not nitpick formatting unless it affects correctness. Do not reproduce the entire diff. Rank findings by severity and cite files/symbols when possible."#;

const DOC_MAP_SYSTEM: &str = r#"You are mapping one chunk of a software repository for a later documentation synthesis step.
Extract only evidence useful for accurate project documentation: responsibilities, entry points, important modules and symbols, data/control flow, APIs, configuration, persistence, integrations, build/deploy behavior, and notable constraints.
Cite concrete file paths and symbols. Do not invent missing behavior. Do not write the final project documentation yet. Keep the chunk summary compact and structured; target roughly 800-1000 tokens and prioritize evidence over prose."#;

const DOC_REDUCE_SYSTEM: &str = r#"You are a senior technical writer synthesizing project documentation from repository-map summaries produced from local source code.
Write documentation that is useful to developers and grounded only in the supplied evidence. Clearly mark unknowns instead of guessing. Prefer concrete file paths, modules, symbols, commands, and data flows. Mermaid diagrams are welcome when relationships are supported by evidence.
When asked for multiple documents, emit each document as a complete Markdown block delimited exactly by:
===== DOCUMENT: relative/path.md =====
...content...
===== END DOCUMENT =====
Do not claim files were written; the MCP is read-only."#;

const UPDATE_DOCS_SYSTEM: &str = r#"You are updating software project documentation from a Git diff and the current documentation snapshot.
Identify only documentation affected by the code changes. Preserve valid existing information, change stale sections, add genuinely new behavior, and avoid speculative statements.
Return complete replacement Markdown only for documents that should change, each delimited exactly by:
===== DOCUMENT: relative/path.md =====
...content...
===== END DOCUMENT =====
If no documentation change is needed, say so explicitly. Do not claim files were written; the MCP is read-only."#;

pub fn run(workspace_path: &Path) -> Result<()> {
    let config = config::load().context("failed to load LLM2MCP configuration")?;
    let workspace = workspace_path
        .canonicalize()
        .with_context(|| format!("invalid workspace: {}", workspace_path.display()))?;
    let _ = jobs::cleanup_expired();

    eprintln!("LLM2MCP MCP started for {}", workspace.display());

    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) => {
                eprintln!("invalid MCP JSON: {error}");
                continue;
            }
        };

        let Some(id) = request.get("id").cloned() else {
            continue;
        };
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let result: std::result::Result<Value, RpcFailure> = match method {
            "initialize" => Ok(initialize_result(&request)),
            "server/discover" => Ok(server_discover_result()),
            "ping" => Ok(json!({"resultType": "complete"})),
            "tools/list" => Ok(tools_list()),
            "tools/call" => {
                call_tool(&config, &workspace, request.get("params")).map_err(RpcFailure::internal)
            }
            "tasks/get" => tasks_get(request.get("params")),
            "tasks/cancel" => tasks_cancel(request.get("params")),
            "tasks/update" => tasks_update(request.get("params")),
            _ => Err(RpcFailure::new(
                -32601,
                format!("method not found: {method}"),
            )),
        };

        let response = match result {
            Ok(mut result) => {
                stamp_modern_server_info(&request, &mut result);
                json!({"jsonrpc": "2.0", "id": id, "result": result})
            }
            Err(error) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": error.code, "message": error.message}
            }),
        };
        serde_json::to_writer(&mut stdout, &response)?;
        stdout.write_all(b"\n")?;
        stdout.flush()?;
    }
    Ok(())
}

pub fn run_job_worker(job_id: &str) -> Result<()> {
    let record = jobs::load(job_id)?;
    if record.cancel_requested || record.state == JobState::Cancelled {
        jobs::mark_cancelled(job_id)?;
        return Ok(());
    }

    let config = config::load().context("failed to load LLM2MCP configuration")?;
    let reporter = Reporter::new(job_id);
    reporter.update("starting", 0, 0)?;

    let result = execute_business_inner(
        &config,
        &record.workspace,
        &record.tool,
        &record.args,
        Some(&reporter),
    );

    match result {
        Ok(text) => {
            if jobs::load(job_id)?.cancel_requested {
                jobs::mark_cancelled(job_id)?;
            } else {
                jobs::complete(job_id, tool_result(text, false))?;
            }
        }
        Err(error) if error.to_string().contains("job cancelled") => {
            jobs::mark_cancelled(job_id)?;
        }
        Err(error) => {
            jobs::fail(job_id, &format!("LLM2MCP error: {error:#}"))?;
        }
    }
    Ok(())
}

fn initialize_result(request: &Value) -> Value {
    let requested = request
        .pointer("/params/protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or(LEGACY_PROTOCOL_VERSION);
    let protocol = if requested.is_empty() || requested.starts_with("2026-") {
        LEGACY_PROTOCOL_VERSION
    } else {
        requested
    };
    json!({
        "protocolVersion": protocol,
        "capabilities": {"tools": {"listChanged": false}},
        "serverInfo": {"name": "LLM2MCP", "version": env!("CARGO_PKG_VERSION")},
        "instructions": server_instructions()
    })
}

fn server_discover_result() -> Value {
    json!({
        "resultType": "complete",
        "supportedVersions": [MODERN_PROTOCOL_VERSION, LEGACY_PROTOCOL_VERSION],
        "capabilities": {
            "tools": {},
            "extensions": {
                TASKS_EXTENSION: {}
            }
        },
        "instructions": server_instructions()
    })
}

#[derive(Debug)]
struct RpcFailure {
    code: i64,
    message: String,
}

impl RpcFailure {
    fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn internal(error: anyhow::Error) -> Self {
        Self::new(-32603, error.to_string())
    }
}

fn is_modern_request(request: &Value) -> bool {
    request
        .pointer("/params/_meta/io.modelcontextprotocol~1protocolVersion")
        .and_then(Value::as_str)
        .is_some_and(|version| version == MODERN_PROTOCOL_VERSION)
}

fn stamp_modern_server_info(request: &Value, result: &mut Value) {
    if !is_modern_request(request) {
        return;
    }
    let Some(object) = result.as_object_mut() else {
        return;
    };
    let meta = object
        .entry("_meta")
        .or_insert_with(|| json!({}))
        .as_object_mut();
    if let Some(meta) = meta {
        meta.insert(
            "io.modelcontextprotocol/serverInfo".to_owned(),
            json!({"name": "LLM2MCP", "version": env!("CARGO_PKG_VERSION")}),
        );
    }
}

fn server_instructions() -> &'static str {
    "Use LLM2MCP to offload large local code analysis, planning, diff review, repository documentation, and documentation maintenance to the configured LLM. Long-running operations may return a durable background job. Prefer passing file or directory paths instead of reading large files into the primary model first."
}

fn tools_list() -> Value {
    json!({
        "resultType": "complete",
        "tools": [
            {
                "name": "analyze",
                "description": "Analyze local files/directories with the configured LLM. May run as a background job when the configured execution mode is Auto/Async.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "task": {"type": "string", "description": "What to analyze and what questions to answer"},
                        "paths": {"type": "array", "items": {"type": "string"}, "description": "Workspace-relative files or directories. Empty means project overview files only."}
                    },
                    "required": ["task"],
                    "additionalProperties": false
                },
                "annotations": {"readOnlyHint": true, "destructiveHint": false}
            },
            {
                "name": "plan",
                "description": "Ask the configured LLM to independently plan an implementation using local project context. May run as a background job.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "task": {"type": "string"},
                        "paths": {"type": "array", "items": {"type": "string"}, "description": "Workspace-relative files or directories"}
                    },
                    "required": ["task"],
                    "additionalProperties": false
                },
                "annotations": {"readOnlyHint": true, "destructiveHint": false}
            },
            {
                "name": "review_diff",
                "description": "Review the current local git diff with the configured LLM without sending the diff through the primary model first.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "task": {"type": "string", "default": "Review the current changes for correctness and regressions"},
                        "base_ref": {"type": "string", "default": "HEAD"}
                    },
                    "additionalProperties": false
                },
                "annotations": {"readOnlyHint": true, "destructiveHint": false}
            },
            {
                "name": "document_repo",
                "description": "Scan a local repository in bounded chunks and synthesize grounded project documentation. Defaults to asynchronous execution because local LLMs may take minutes.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "document_type": {"type": "string", "enum": ["overview", "architecture", "modules", "api", "developer", "deployment", "full"], "default": "overview"},
                        "paths": {"type": "array", "items": {"type": "string"}, "description": "Optional workspace-relative files/directories to document. Empty scans the repository."},
                        "audience": {"type": "string", "default": "developer"},
                        "language": {"type": "string", "enum": ["auto", "english", "simplified_chinese"], "default": "auto"}
                    },
                    "additionalProperties": false
                },
                "annotations": {"readOnlyHint": true, "destructiveHint": false}
            },
            {
                "name": "update_docs",
                "description": "Compare code changes with existing README/docs and return replacement Markdown only for documentation that should change. Defaults to asynchronous execution.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "base_ref": {"type": "string", "default": "HEAD"},
                        "target_ref": {"type": "string", "default": "WORKTREE", "description": "Git ref to compare against base_ref, or WORKTREE for current staged/unstaged tracked changes."},
                        "docs": {"type": "array", "items": {"type": "string"}, "description": "Optional workspace-relative Markdown docs. Empty auto-discovers README and docs/**/*.md."},
                        "language": {"type": "string", "enum": ["auto", "english", "simplified_chinese"], "default": "auto"}
                    },
                    "additionalProperties": false
                },
                "annotations": {"readOnlyHint": true, "destructiveHint": false}
            },
            {
                "name": "job_status",
                "description": "Get status and progress for an LLM2MCP background job returned by an asynchronous tool call.",
                "inputSchema": {
                    "type": "object",
                    "properties": {"job_id": {"type": "string"}},
                    "required": ["job_id"],
                    "additionalProperties": false
                },
                "annotations": {"readOnlyHint": true, "destructiveHint": false}
            },
            {
                "name": "job_result",
                "description": "Retrieve the final result of an LLM2MCP background job. While still running, returns the current status instead.",
                "inputSchema": {
                    "type": "object",
                    "properties": {"job_id": {"type": "string"}},
                    "required": ["job_id"],
                    "additionalProperties": false
                },
                "annotations": {"readOnlyHint": true, "destructiveHint": false}
            },
            {
                "name": "job_cancel",
                "description": "Request cooperative cancellation of an LLM2MCP background job.",
                "inputSchema": {
                    "type": "object",
                    "properties": {"job_id": {"type": "string"}},
                    "required": ["job_id"],
                    "additionalProperties": false
                },
                "annotations": {"readOnlyHint": true, "destructiveHint": false}
            }
        ]
    })
}

fn call_tool(config: &AppConfig, root: &Path, params: Option<&Value>) -> Result<Value> {
    let params = params.context("missing tools/call params")?;
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .context("missing tool name")?;
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    match name {
        "job_status" => return job_status(&args),
        "job_result" => return job_result(&args),
        "job_cancel" => return job_cancel(&args),
        _ => {}
    }

    let tool = tool_config(config, name)?;
    let execution = effective_execution(name, tool, root, &args);
    if execution == ExecutionMode::Async {
        let record = jobs::create(name, root, args)?;
        if client_supports_tasks(params) {
            return Ok(jobs::create_task_result(&record));
        }
        return Ok(jobs::fallback_started_result(&record));
    }

    Ok(
        match execute_business_inner(config, root, name, &args, None) {
            Ok(text) => tool_result(text, false),
            Err(error) => {
                eprintln!("tool {name} failed: {error:#}");
                tool_result(format!("LLM2MCP error: {error:#}"), true)
            }
        },
    )
}

fn tool_config<'a>(config: &'a AppConfig, name: &str) -> Result<&'a ToolConfig> {
    match name {
        "analyze" => Ok(&config.tools.analyze),
        "plan" => Ok(&config.tools.plan),
        "review_diff" => Ok(&config.tools.review_diff),
        "document_repo" => Ok(&config.tools.document_repo),
        "update_docs" => Ok(&config.tools.update_docs),
        _ => bail!("unknown tool: {name}"),
    }
}

fn default_execution(name: &str) -> ExecutionMode {
    match name {
        "review_diff" => ExecutionMode::Sync,
        "document_repo" | "update_docs" => ExecutionMode::Async,
        _ => ExecutionMode::Auto,
    }
}

fn effective_execution(name: &str, tool: &ToolConfig, root: &Path, args: &Value) -> ExecutionMode {
    match tool.execution_or(default_execution(name)) {
        ExecutionMode::Sync => ExecutionMode::Sync,
        ExecutionMode::Async => ExecutionMode::Async,
        ExecutionMode::Auto => {
            if auto_should_async(name, root, args) {
                ExecutionMode::Async
            } else {
                ExecutionMode::Sync
            }
        }
    }
}

fn auto_should_async(name: &str, root: &Path, args: &Value) -> bool {
    if matches!(name, "document_repo" | "update_docs") {
        return true;
    }
    if name == "review_diff" {
        return false;
    }
    let paths = paths_arg(args);
    if paths.len() > 1 {
        return true;
    }
    paths.iter().any(|relative| {
        let path = root.join(relative);
        path.is_dir()
            || path
                .metadata()
                .map(|metadata| metadata.len() > 80_000)
                .unwrap_or(false)
    })
}

fn client_supports_tasks(params: &Value) -> bool {
    let Some(meta) = params.get("_meta").and_then(Value::as_object) else {
        return false;
    };
    let modern = meta
        .get("io.modelcontextprotocol/protocolVersion")
        .and_then(Value::as_str)
        .is_some_and(|version| version == MODERN_PROTOCOL_VERSION);
    let tasks = meta
        .get("io.modelcontextprotocol/clientCapabilities")
        .and_then(Value::as_object)
        .and_then(|caps| caps.get("extensions"))
        .and_then(Value::as_object)
        .is_some_and(|extensions| extensions.contains_key(TASKS_EXTENSION));
    modern && tasks
}

fn require_tasks_params<'a>(
    params: Option<&'a Value>,
    method: &str,
) -> std::result::Result<&'a Value, RpcFailure> {
    let params =
        params.ok_or_else(|| RpcFailure::new(-32602, format!("{method} requires params")))?;
    if !client_supports_tasks(params) {
        return Err(RpcFailure::new(
            -32003,
            format!("{method} requires the {TASKS_EXTENSION} client capability"),
        ));
    }
    Ok(params)
}

fn task_id_from_params<'a>(
    params: &'a Value,
    method: &str,
) -> std::result::Result<&'a str, RpcFailure> {
    params
        .get("taskId")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| RpcFailure::new(-32602, format!("{method} requires taskId")))
}

fn load_task_for_rpc(job_id: &str) -> std::result::Result<jobs::JobRecord, RpcFailure> {
    jobs::load(job_id).map_err(|error| RpcFailure::new(-32602, error.to_string()))
}

fn tasks_get(params: Option<&Value>) -> std::result::Result<Value, RpcFailure> {
    let params = require_tasks_params(params, "tasks/get")?;
    let job_id = task_id_from_params(params, "tasks/get")?;
    Ok(jobs::task_get_result(&load_task_for_rpc(job_id)?))
}

fn tasks_cancel(params: Option<&Value>) -> std::result::Result<Value, RpcFailure> {
    let params = require_tasks_params(params, "tasks/cancel")?;
    let job_id = task_id_from_params(params, "tasks/cancel")?;
    load_task_for_rpc(job_id)?;
    jobs::cancel(job_id).map_err(RpcFailure::internal)?;
    Ok(json!({"resultType": "complete"}))
}

fn tasks_update(params: Option<&Value>) -> std::result::Result<Value, RpcFailure> {
    let params = require_tasks_params(params, "tasks/update")?;
    let job_id = task_id_from_params(params, "tasks/update")?;
    load_task_for_rpc(job_id)?;
    // LLM2MCP jobs currently never enter input_required, so there are no
    // outstanding input requests to satisfy. Unknown responses are ignored.
    Ok(json!({"resultType": "complete"}))
}

fn job_id_arg(args: &Value) -> Result<&str> {
    args.get("job_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .context("job_id is required")
}

fn job_status(args: &Value) -> Result<Value> {
    Ok(jobs::fallback_status_result(&jobs::load(job_id_arg(
        args,
    )?)?))
}

fn job_result(args: &Value) -> Result<Value> {
    Ok(jobs::fallback_result(&jobs::load(job_id_arg(args)?)?))
}

fn job_cancel(args: &Value) -> Result<Value> {
    let record = jobs::cancel(job_id_arg(args)?)?;
    Ok(jobs::fallback_status_result(&record))
}

fn tool_result(text: String, is_error: bool) -> Value {
    json!({
        "resultType": "complete",
        "content": [{"type": "text", "text": text}],
        "isError": is_error
    })
}

fn execute_business_inner(
    config: &AppConfig,
    root: &Path,
    name: &str,
    args: &Value,
    reporter: Option<&Reporter>,
) -> Result<String> {
    match name {
        "analyze" => analyze(config, root, args, &config.tools.analyze, reporter),
        "plan" => plan(config, root, args, &config.tools.plan, reporter),
        "review_diff" => review_diff(config, root, args, &config.tools.review_diff, reporter),
        "document_repo" => document_repo(config, root, args, &config.tools.document_repo, reporter),
        "update_docs" => update_docs(config, root, args, &config.tools.update_docs, reporter),
        _ => bail!("unknown background tool: {name}"),
    }
}

fn paths_arg(args: &Value) -> Vec<String> {
    args.get("paths")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn task_arg(args: &Value) -> Result<&str> {
    args.get("task")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .context("task is required")
}

fn progress(reporter: Option<&Reporter>, stage: &str, done: u64, total: u64) -> Result<()> {
    if let Some(reporter) = reporter {
        reporter.update(stage, done, total)?;
    }
    Ok(())
}

fn analyze(
    config: &AppConfig,
    root: &Path,
    args: &Value,
    tool: &ToolConfig,
    reporter: Option<&Reporter>,
) -> Result<String> {
    progress(reporter, "collecting context", 0, 2)?;
    let task = task_arg(args)?;
    let sources = workspace::collect(root, &paths_arg(args), config)?;
    progress(reporter, "analyzing", 1, 2)?;
    let prompt = format!(
        "TASK\n{task}\n\nREPOSITORY MANIFEST\n{}\n\nFILES INCLUDED\n{}\n\nSOURCE MATERIAL\n{}\n\nSOURCE_TRUNCATED\n{}\n\nReturn: 1) executive conclusion, 2) key evidence, 3) root causes/risks, 4) recommended actions, 5) files the primary agent should inspect next.",
        sources.manifest,
        sources.included_files.join("\n"),
        sources.body,
        sources.truncated,
    );
    let result = llm::chat(
        config,
        ANALYZE_SYSTEM,
        &prompt,
        tool.reasoning,
        tool.max_output_tokens,
    )?;
    progress(reporter, "finishing", 2, 2)?;
    Ok(result)
}

fn plan(
    config: &AppConfig,
    root: &Path,
    args: &Value,
    tool: &ToolConfig,
    reporter: Option<&Reporter>,
) -> Result<String> {
    progress(reporter, "collecting context", 0, 2)?;
    let task = task_arg(args)?;
    let sources = workspace::collect(root, &paths_arg(args), config)?;
    progress(reporter, "planning", 1, 2)?;
    let prompt = format!(
        "TASK\n{task}\n\nREPOSITORY MANIFEST\n{}\n\nFILES INCLUDED\n{}\n\nSOURCE MATERIAL\n{}\n\nSOURCE_TRUNCATED\n{}\n\nProduce: goal/constraints, proposed implementation, files/modules to change, ordered steps, risks, and tests.",
        sources.manifest,
        sources.included_files.join("\n"),
        sources.body,
        sources.truncated,
    );
    let result = llm::chat(
        config,
        PLAN_SYSTEM,
        &prompt,
        tool.reasoning,
        tool.max_output_tokens,
    )?;
    progress(reporter, "finishing", 2, 2)?;
    Ok(result)
}

fn review_diff(
    config: &AppConfig,
    root: &Path,
    args: &Value,
    tool: &ToolConfig,
    reporter: Option<&Reporter>,
) -> Result<String> {
    progress(reporter, "reading git diff", 0, 2)?;
    let task = args
        .get("task")
        .and_then(Value::as_str)
        .unwrap_or("Review the current changes for correctness and regressions");
    let base_ref = args
        .get("base_ref")
        .and_then(Value::as_str)
        .unwrap_or("HEAD");
    let (status, diff, truncated) = workspace::git_diff(root, base_ref, config.max_source_chars)?;
    if diff.trim().is_empty() {
        return Ok("No tracked git diff found for review.".to_owned());
    }
    progress(reporter, "reviewing", 1, 2)?;
    let prompt = format!(
        "REVIEW TASK\n{task}\n\nGIT STATUS\n{status}\n\nDIFF AGAINST\n{base_ref}\n\nDIFF\n{diff}\n\nDIFF_TRUNCATED\n{truncated}\n\nReturn critical/high/medium findings, missing tests, and recommended fixes.",
    );
    let result = llm::chat(
        config,
        REVIEW_SYSTEM,
        &prompt,
        tool.reasoning,
        tool.max_output_tokens,
    )?;
    progress(reporter, "finishing", 2, 2)?;
    Ok(result)
}

fn string_list_arg(args: &Value, name: &str) -> Vec<String> {
    args.get(name)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn enum_arg<'a>(
    args: &'a Value,
    name: &str,
    default: &'a str,
    allowed: &[&str],
) -> Result<&'a str> {
    let value = args.get(name).and_then(Value::as_str).unwrap_or(default);
    if allowed.contains(&value) {
        Ok(value)
    } else {
        bail!("invalid {name}: {value}")
    }
}

fn document_targets(document_type: &str) -> &'static str {
    match document_type {
        "overview" => "docs/PROJECT_OVERVIEW.md",
        "architecture" => "docs/ARCHITECTURE.md",
        "modules" => "docs/MODULES.md",
        "api" => "docs/API.md",
        "developer" => "docs/DEVELOPMENT.md",
        "deployment" => "docs/DEPLOYMENT.md",
        "full" => {
            "docs/PROJECT_OVERVIEW.md\ndocs/ARCHITECTURE.md\ndocs/MODULES.md\ndocs/API.md\ndocs/DEVELOPMENT.md\ndocs/DEPLOYMENT.md"
        }
        _ => unreachable!("document type is validated before this call"),
    }
}

fn map_repository_chunk(
    config: &AppConfig,
    chunk: &workspace::CollectedSource,
    index: usize,
    total_chunks: usize,
    map_tokens: u32,
) -> Result<String> {
    let prompt = format!(
        "CHUNK {}/{}\n\nFILES\n{}\n\nSOURCE\n{}\n\nExtract repository-map evidence for later documentation synthesis. Focus on responsibilities, entry points, important symbols, flows, APIs, configuration, build/deploy behavior, integrations, and constraints. Cite file paths and symbols. Keep the result concise; this summary will be combined with other chunks later.\n",
        index + 1,
        total_chunks,
        chunk.included_files.join("\n"),
        chunk.body,
    );
    llm::chat(
        config,
        DOC_MAP_SYSTEM,
        &prompt,
        ReasoningEffort::Low,
        map_tokens,
    )
}

fn reduce_reasoning_attempts(configured: ReasoningEffort) -> Vec<ReasoningEffort> {
    match configured {
        ReasoningEffort::Xhigh => vec![
            ReasoningEffort::Xhigh,
            ReasoningEffort::Medium,
            ReasoningEffort::Low,
        ],
        ReasoningEffort::Medium => vec![ReasoningEffort::Medium, ReasoningEffort::Low],
        ReasoningEffort::Low => vec![ReasoningEffort::Low],
        ReasoningEffort::Off => vec![ReasoningEffort::Off],
    }
}

fn document_repo(
    config: &AppConfig,
    root: &Path,
    args: &Value,
    tool: &ToolConfig,
    reporter: Option<&Reporter>,
) -> Result<String> {
    const MAX_CHUNKS: usize = 12;
    const MAX_CHUNK_CHARS: usize = 160_000;

    progress(reporter, "scanning repository", 0, 1)?;
    let document_type = enum_arg(
        args,
        "document_type",
        "overview",
        &[
            "overview",
            "architecture",
            "modules",
            "api",
            "developer",
            "deployment",
            "full",
        ],
    )?;
    let language = enum_arg(
        args,
        "language",
        "auto",
        &["auto", "english", "simplified_chinese"],
    )?;
    let audience = args
        .get("audience")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("developer");
    let paths = paths_arg(args);
    let chunks = workspace::collect_chunks(
        root,
        &paths,
        config,
        config.max_source_chars.min(MAX_CHUNK_CHARS),
        MAX_CHUNKS,
    )?;
    if chunks.is_empty() {
        bail!("no readable source files found for documentation");
    }

    // Repository-map summaries are evidence extraction, not final prose. Keeping
    // them compact dramatically reduces both map time and final Reduce context.
    let map_tokens = (tool.max_output_tokens / 10).clamp(800, 1_200);
    let total_chunks = chunks.len();
    let total_steps = total_chunks as u64 + 1;
    let scan_truncated = chunks.iter().any(|chunk| chunk.truncated);
    let manifest = chunks[0].manifest.clone();
    let concurrency = config
        .document_map_concurrency
        .clamp(1, 8)
        .min(total_chunks.max(1));

    let mut summaries: Vec<Option<String>> = vec![None; total_chunks];
    let mut pending = Vec::new();
    let mut completed = 0usize;

    for (index, chunk) in chunks.iter().enumerate() {
        let key = doc_cache::map_key(config, map_tokens, chunk);
        match doc_cache::load_summary(&key) {
            Ok(Some(summary)) => {
                summaries[index] = Some(summary);
                completed += 1;
                eprintln!(
                    "LLM2MCP document_repo map cache hit {}/{} ({} files)",
                    index + 1,
                    total_chunks,
                    chunk.included_files.len()
                );
            }
            Ok(None) => pending.push((index, key)),
            Err(error) => {
                eprintln!(
                    "LLM2MCP map cache read failed for chunk {}: {error:#}",
                    index + 1
                );
                pending.push((index, key));
            }
        }
    }

    progress(
        reporter,
        if completed > 0 {
            "mapping repository (checkpoint cache)"
        } else {
            "mapping repository"
        },
        completed as u64,
        total_steps,
    )?;

    for batch in pending.chunks(concurrency) {
        let batch_results: Result<Vec<(usize, String)>> = thread::scope(|scope| {
            let mut handles = Vec::with_capacity(batch.len());
            for (index, key) in batch {
                let index = *index;
                let key = key.clone();
                let chunk = &chunks[index];
                handles.push(scope.spawn(move || -> Result<(usize, String)> {
                    eprintln!(
                        "LLM2MCP document_repo mapping chunk {}/{} ({} files)",
                        index + 1,
                        total_chunks,
                        chunk.included_files.len()
                    );
                    let summary =
                        map_repository_chunk(config, chunk, index, total_chunks, map_tokens)?;
                    if let Err(error) = doc_cache::store_summary(&key, &summary) {
                        eprintln!(
                            "LLM2MCP map checkpoint write failed for chunk {}: {error:#}",
                            index + 1
                        );
                    }
                    Ok((index, summary))
                }));
            }

            let mut results = Vec::with_capacity(handles.len());
            for handle in handles {
                let result = handle
                    .join()
                    .map_err(|_| anyhow::anyhow!("repository map worker thread panicked"))??;
                results.push(result);
            }
            Ok(results)
        });

        for (index, summary) in batch_results? {
            summaries[index] = Some(summary);
            completed += 1;
            progress(
                reporter,
                "mapping repository",
                completed as u64,
                total_steps,
            )?;
        }
    }

    let summaries = summaries
        .into_iter()
        .enumerate()
        .map(|(index, summary)| {
            summary
                .map(|summary| {
                    format!(
                        "===== MAP SUMMARY {}/{} =====\n{}\n===== END MAP SUMMARY =====",
                        index + 1,
                        total_chunks,
                        summary
                    )
                })
                .with_context(|| format!("missing repository map summary for chunk {}", index + 1))
        })
        .collect::<Result<Vec<_>>>()?;

    let base_prompt = format!(
        "DOCUMENT TYPE\n{document_type}\n\nTARGET DOCUMENTS\n{}\n\nAUDIENCE\n{audience}\n\nOUTPUT LANGUAGE\n{language}\n\nREPOSITORY MANIFEST\n{manifest}\n\nREPOSITORY MAP SUMMARIES\n{}\n\nSCAN_TRUNCATED\n{scan_truncated}\n\nSynthesize the requested documentation. For full mode, return every target document as a complete Markdown document. For a single document type, return exactly that target document. Ground claims in the repository evidence and mark unknowns instead of guessing. Emit the final Markdown directly; do not spend the output budget restating your analysis.",
        document_targets(document_type),
        summaries.join("\n\n"),
    );

    let attempts = reduce_reasoning_attempts(tool.reasoning);
    let mut last_diagnostics = String::from("no response received");
    for (attempt_index, effort) in attempts.iter().copied().enumerate() {
        let stage = if attempt_index == 0 {
            "synthesizing documentation"
        } else {
            "retrying synthesis with lower reasoning"
        };
        progress(reporter, stage, total_chunks as u64, total_steps)?;

        let prompt = if attempt_index == 0 {
            base_prompt.clone()
        } else {
            format!(
                "{base_prompt}\n\nSYNTHESIS RETRY DIRECTIVE\nThe previous synthesis attempt exhausted or failed to produce final message content. Do not perform extended reasoning. Use the existing repository-map evidence and emit the requested final Markdown immediately."
            )
        };
        let response = llm::chat_detailed(
            config,
            DOC_REDUCE_SYSTEM,
            &prompt,
            effort,
            tool.max_output_tokens,
        )?;
        if let Some(content) = response.final_text() {
            progress(reporter, "finishing", total_steps, total_steps)?;
            return Ok(content.trim().to_owned());
        }

        last_diagnostics = response.diagnostics();
        eprintln!(
            "LLM2MCP document_repo Reduce attempt {} ({:?}) returned no final content: {}",
            attempt_index + 1,
            effort,
            last_diagnostics
        );
        if !response.exhausted_before_final() {
            eprintln!(
                "LLM2MCP Reduce response was empty without a clear token-exhaustion signal; trying the next lower reasoning level if available"
            );
        }
    }

    let _ = progress(reporter, "reduce_failed", total_chunks as u64, total_steps);
    bail!(
        "document synthesis produced no final Markdown after {} reasoning attempt(s); last response: {}",
        attempts.len(),
        last_diagnostics
    )
}

fn update_docs(
    config: &AppConfig,
    root: &Path,
    args: &Value,
    tool: &ToolConfig,
    reporter: Option<&Reporter>,
) -> Result<String> {
    progress(reporter, "reading code changes", 0, 3)?;
    let language = enum_arg(
        args,
        "language",
        "auto",
        &["auto", "english", "simplified_chinese"],
    )?;
    let base_ref = args
        .get("base_ref")
        .and_then(Value::as_str)
        .unwrap_or("HEAD");
    let target_ref = args
        .get("target_ref")
        .and_then(Value::as_str)
        .unwrap_or("WORKTREE");

    let per_side_chars = (config.max_source_chars / 2).max(10_000);
    let (diff, diff_truncated, comparison) = if target_ref.eq_ignore_ascii_case("WORKTREE") {
        let (_status, diff, truncated) = workspace::git_diff(root, base_ref, per_side_chars)
            .context("update_docs requires a Git repository with a valid base_ref")?;
        (diff, truncated, format!("{base_ref} -> WORKTREE"))
    } else {
        let (diff, truncated) =
            workspace::git_diff_refs(root, base_ref, target_ref, per_side_chars)
                .context("update_docs requires a Git repository with valid base_ref/target_ref")?;
        (diff, truncated, format!("{base_ref} -> {target_ref}"))
    };

    if diff.trim().is_empty() {
        return Ok(format!(
            "No code changes found for {comparison}; documentation is unchanged."
        ));
    }

    progress(reporter, "reading existing documentation", 1, 3)?;
    let requested_docs = string_list_arg(args, "docs");
    let doc_paths = if requested_docs.is_empty() {
        workspace::existing_document_paths(root)?
    } else {
        requested_docs
    };
    let existing_docs = if doc_paths.is_empty() {
        None
    } else {
        let mut docs_config = config.clone();
        docs_config.max_source_chars = per_side_chars;
        Some(workspace::collect(root, &doc_paths, &docs_config)?)
    };

    let (doc_list, doc_body, docs_truncated) = if let Some(docs) = existing_docs {
        (docs.included_files.join("\n"), docs.body, docs.truncated)
    } else {
        (
            "No existing README/docs Markdown files were discovered.".to_owned(),
            String::new(),
            false,
        )
    };

    progress(reporter, "updating documentation", 2, 3)?;
    let prompt = format!(
        "COMPARISON\n{comparison}\n\nOUTPUT LANGUAGE\n{language}\n\nCODE DIFF\n{diff}\n\nDIFF_TRUNCATED\n{diff_truncated}\n\nEXISTING DOCUMENTS\n{doc_list}\n\nDOCUMENT CONTENT\n{doc_body}\n\nDOCS_TRUNCATED\n{docs_truncated}\n\nDetermine which documents are affected by the code changes. Return complete replacement Markdown only for documents that actually need changes. If a new document is clearly required, use a docs/*.md path and explain it through the document content itself.",
    );
    let result = llm::chat(
        config,
        UPDATE_DOCS_SYSTEM,
        &prompt,
        tool.reasoning,
        tool.max_output_tokens,
    )?;
    progress(reporter, "finishing", 3, 3)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduce_reasoning_falls_back_without_repeating_levels() {
        assert_eq!(
            reduce_reasoning_attempts(ReasoningEffort::Xhigh),
            vec![
                ReasoningEffort::Xhigh,
                ReasoningEffort::Medium,
                ReasoningEffort::Low
            ]
        );
        assert_eq!(
            reduce_reasoning_attempts(ReasoningEffort::Medium),
            vec![ReasoningEffort::Medium, ReasoningEffort::Low]
        );
    }

    #[test]
    fn tools_list_includes_job_fallback_tools() {
        let listing = tools_list();
        let tools = listing["tools"].as_array().expect("tools array");
        assert_eq!(tools.len(), 8);
        assert!(tools.iter().any(|tool| tool["name"] == "job_status"));
        assert!(tools.iter().any(|tool| tool["name"] == "document_repo"));
    }

    #[test]
    fn tasks_extension_requires_modern_per_request_opt_in() {
        let params = json!({
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": MODERN_PROTOCOL_VERSION,
                "io.modelcontextprotocol/clientCapabilities": {
                    "extensions": {TASKS_EXTENSION: {}}
                }
            }
        });
        assert!(client_supports_tasks(&params));
        assert!(!client_supports_tasks(&json!({})));

        let missing = tasks_get(Some(&json!({"taskId": "job_test"})))
            .expect_err("tasks/get must reject a client that did not opt into Tasks");
        assert_eq!(missing.code, -32003);
    }

    #[test]
    fn modern_results_stamp_server_info_in_meta() {
        let request = json!({
            "params": {
                "_meta": {
                    "io.modelcontextprotocol/protocolVersion": MODERN_PROTOCOL_VERSION
                }
            }
        });
        let mut result = server_discover_result();
        stamp_modern_server_info(&request, &mut result);
        assert!(result.get("serverInfo").is_none());
        assert_eq!(
            result["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
            "LLM2MCP"
        );
    }
}
