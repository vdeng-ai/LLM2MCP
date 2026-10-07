#[path = "supplement.rs"]
mod supplement;
use supplement::{AnalysisContext, analyze_with_supplement};

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
Return JSON only, with no Markdown or code fences, in this shape:
{"conclusion":"...","findings":[{"severity":"high|medium|low|info","text":"...","evidence":["path:line-range or symbol"]}],"risks":["..."],"actions":["..."],"read_next":[{"path":"relative/path","symbol":"exact symbol label if known","lines":"start-end","reason":"..."}]}.
Keep fields concise; the bridge will locally compact this before returning it to the primary agent.
When supplied evidence is insufficient, you may include optional "context_requests":[{"query":"exact identifier or question","paths":["existing relative/path"]}] alongside the required result fields. Request at most three targeted searches. The bridge permits one local supplemental round and one further analysis pass; do not invent paths or request commands."#;

const DEBUG_SYSTEM: &str = r#"You are a local software-debugging specialist assisting a primary coding agent.
Diagnose the reported malfunction from the supplied symptoms, logs, repository evidence, and optional recent Git changes.
Rules:
- Focus on why the system is failing, not on redesigning unrelated code.
- Trace the likely execution path from symptom to failure when evidence permits.
- Separate confirmed evidence from hypotheses and assign an explicit confidence level.
- Explain intermittent behavior when relevant, including races, state, timing, retries, caching, environment, or nondeterminism.
- Prefer exact file paths, symbols, and supplied line ranges.
- Do not invent stack frames, paths, line ranges, runtime results, or commands that were not supplied.
- Suggest verification steps and likely fix areas, but do not claim you ran commands or modified files.
- Do not reveal hidden chain-of-thought.
Return JSON only, with no Markdown or code fences, in this shape:
{"diagnosis":"...","confidence":"high|medium|low","root_cause":"...","evidence":[{"severity":"high|medium|low|info","text":"...","evidence":["path:start-end, log:exact supplied excerpt, or diff:path:exact supplied hunk/excerpt"]}],"execution_path":["..."],"intermittency":"...","alternatives":["..."],"verification":["..."],"fix_area":["..."],"read_next":[{"path":"relative/path","symbol":"exact symbol label if known","lines":"start-end","reason":"..."}]}.
Keep fields concise; the bridge will locally validate read_next and compact the result before returning it to the primary agent.
When supplied evidence is insufficient, you may include optional "context_requests":[{"query":"exact identifier or question","paths":["existing relative/path"]}] alongside the required result fields. Request at most three targeted searches. The bridge permits one local supplemental round and one further analysis pass; do not invent paths or request commands."#;

const PLAN_SYSTEM: &str = r#"You are a local software-planning specialist assisting a primary coding agent.
Analyze the supplied repository information and produce an implementation plan.
Rules:
- Do not repeat the source material.
- Preserve important constraints.
- Mention concrete files, modules, and symbols.
- Prefer the smallest implementation that satisfies the task.
- Identify dependencies, risks, compatibility concerns, and verification steps.
- Do not claim changes have already been made.
- Do not reveal hidden chain-of-thought.
Return JSON only, with no Markdown or code fences, in this shape:
{"goal":"...","steps":["..."],"risks":["..."],"tests":["..."],"read_next":[{"path":"relative/path","symbol":"exact symbol label if known","lines":"start-end","reason":"..."}]}.
Keep fields concise; the bridge will locally compact this before returning it to the primary agent.
When supplied evidence is insufficient, you may include optional "context_requests":[{"query":"exact identifier or question","paths":["existing relative/path"]}] alongside the required result fields. Request at most three targeted searches. The bridge permits one local supplemental round and one further analysis pass; do not invent paths or request commands."#;

const DISCOVERY_SYSTEM: &str = r#"You are selecting repository symbols for a deeper second-pass analysis by another model call.
Use only the supplied lightweight file/symbol index. Prefer the smallest exact symbol ranges that can answer the task; use whole files only when no suitable symbol range is available.
Return JSON only in this exact shape: {"symbols":[{"path":"relative/path","start_line":1,"end_line":20}],"files":["relative/path"]}.
Choose at most 12 symbol ranges/files total. Copy candidate paths and line ranges exactly. Do not invent paths, line ranges, explanations, Markdown, or code fences."#;

const REVIEW_SYSTEM: &str = r#"You are a local code-review specialist assisting a primary coding agent.
Review the supplied git diff for correctness bugs, regressions, concurrency issues, resource leaks, security problems, API compatibility, and missing tests.
Do not nitpick formatting unless it affects correctness. Do not reproduce the entire diff. Rank findings by severity. Cite supplied numbered source as path:start-end or actual patch text as diff:path:exact hunk/excerpt; copy the excerpt exactly.
Return JSON only, with no Markdown or code fences, in this shape:
{"conclusion":"...","findings":[{"severity":"critical|high|medium|low","text":"...","evidence":["path:start-end or diff:path:exact hunk/excerpt"]}],"tests":["..."],"actions":["..."]}.
Keep fields concise; the bridge will locally compact this before returning it to the primary agent."#;

pub(crate) const DOC_MAP_SYSTEM: &str = r#"You are mapping one chunk of a software repository for a later documentation synthesis step.
Extract only evidence useful for accurate project documentation: responsibilities, entry points, important modules and symbols, data/control flow, APIs, configuration, persistence, integrations, build/deploy behavior, and notable constraints.
Cite concrete file paths and symbols. Do not invent missing behavior. Do not write the final project documentation yet. Keep the chunk summary compact and structured; target roughly 800-1000 tokens and prioritize evidence over prose."#;

const DOC_REDUCE_SYSTEM: &str = r#"You are a senior technical writer synthesizing project documentation from repository-map summaries produced from local source code.
Write documentation that is useful to developers and grounded only in the supplied evidence. Clearly mark unknowns instead of guessing. Prefer concrete file paths, modules, symbols, commands, and data flows. Mermaid diagrams are welcome when relationships are supported by evidence.
For every requested document, emit each document as a complete Markdown block delimited exactly by:
===== DOCUMENT: relative/path.md =====
...content...
===== END DOCUMENT =====
Do not claim files were written; the MCP is read-only."#;

const UPDATE_DOCS_SYSTEM: &str = r#"You are updating software project documentation from a Git diff and the current documentation snapshot.
Identify only documentation affected by the code changes. Preserve valid existing information, change stale sections, add genuinely new behavior, and avoid speculative statements.
Return JSON only: {"edits":[{"path":"README.md","original_sha256":"copy supplied hash","old_text":"exact unique supplied fragment","new_text":"replacement fragment"}],"new_documents":[{"path":"docs/new.md","content":"complete new Markdown"}]}.
Prefer small nonoverlapping exact edits; never replace an entire document from a truncated preview. Preserve unseen sections. Old text must occur entirely within one supplied preview_fragments item's text and match the original exactly once; never combine separate fragments. Use an empty edits array when unchanged. New documents are optional and must not already exist. Do not claim files were written; the MCP is read-only."#;

pub fn run(workspace_path: &Path) -> Result<()> {
    let workspace = workspace_path
        .canonicalize()
        .with_context(|| format!("invalid workspace: {}", workspace_path.display()))?;
    let _ = jobs::recover_stale();
    let _ = jobs::cleanup_expired();
    let active = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::<
        String,
        crate::control::Control,
    >::new()));
    thread::scope(|scope| -> Result<()> {
        for line in io::stdin().lock().lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let request: Value = match serde_json::from_str(&line) {
                Ok(value) => value,
                Err(error) => {
                    send_reply(
                        &json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":error.to_string()}}),
                    )?;
                    continue;
                }
            };
            let method = request
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if method == "notifications/cancelled" {
                if let Some(id) = request.pointer("/params/requestId")
                    && let Some(control) = active.lock().unwrap().get(&id.to_string())
                {
                    control.cancel();
                }
                continue;
            }
            let Some(id) = request.get("id") else {
                continue;
            };
            let business = method == "tools/call"
                && !matches!(
                    request.pointer("/params/name").and_then(Value::as_str),
                    Some("job_status" | "job_result" | "job_cancel" | "result_page")
                );
            if !business {
                send_reply(&rpc_response(&workspace, &request))?;
                continue;
            }
            let key = id.to_string();
            let control = crate::control::Control::default();
            {
                let mut active = active.lock().unwrap();
                if active.len() >= 8 || active.contains_key(&key) {
                    send_reply(
                        &json!({"jsonrpc":"2.0","id":id,"error":{"code":-32000,"message":"server busy or duplicate request id; retry later"}}),
                    )?;
                    continue;
                }
                active.insert(key.clone(), control.clone());
            }
            let active = active.clone();
            let root = &workspace;
            scope.spawn(move || { control.set_current(); let reply = std::panic::catch_unwind(|| rpc_response(root, &request)).unwrap_or_else(|_| json!({"jsonrpc":"2.0","id":request.get("id"),"error":{"code":-32603,"message":"request worker failed"}})); let _ = send_reply(&reply); active.lock().unwrap().remove(&key); });
        }
        for control in active.lock().unwrap().values() {
            control.cancel();
        }
        Ok(())
    })
}
fn send_reply(reply: &Value) -> Result<()> {
    let mut output = io::stdout().lock();
    serde_json::to_writer(&mut output, reply)?;
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(())
}
fn rpc_response(root: &Path, request: &Value) -> Value {
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let result: std::result::Result<Value, RpcFailure> = match method {
        "initialize" => Ok(initialize_result(request)),
        "server/discover" => Ok(server_discover_result()),
        "ping" => Ok(json!({"resultType":"complete"})),
        "tools/list" => Ok(tools_list()),
        "tools/call" => config::load()
            .map_err(RpcFailure::internal)
            .and_then(|config| {
                call_tool(&config, root, request.get("params")).map_err(RpcFailure::internal)
            }),
        "tasks/get" => tasks_get(request.get("params")),
        "tasks/cancel" => tasks_cancel(request.get("params")),
        "tasks/update" => tasks_update(request.get("params")),
        _ => Err(RpcFailure::new(
            -32601,
            format!("method not found: {method}"),
        )),
    };
    match result {
        Ok(mut result) => {
            stamp_modern_server_info(request, &mut result);
            json!({"jsonrpc":"2.0","id":request.get("id"),"result":result})
        }
        Err(error) => {
            json!({"jsonrpc":"2.0","id":request.get("id"),"error":{"code":error.code,"message":crate::privacy::redact(&error.message)}})
        }
    }
}

pub fn run_job_worker(job_id: &str) -> Result<()> {
    let _lease = jobs::worker_lease(job_id)?;
    if let Err(error) = run_job_worker_inner(job_id) {
        jobs::fail(job_id, &format!("LLM2MCP error: {error:#}"))?;
    }
    Ok(())
}

fn run_job_worker_inner(job_id: &str) -> Result<()> {
    let record = jobs::load(job_id)?;
    if record.state != JobState::Working {
        return Ok(());
    }
    let _heartbeat = jobs::start_heartbeat(job_id)?;
    let control = crate::control::Control::default().with_job(job_id);
    control.set_current();
    if record.cancel_requested || record.state == JobState::Cancelled {
        jobs::mark_cancelled(job_id)?;
        return Ok(());
    }

    let current = config::load().context("failed to load LLM2MCP configuration")?;
    let mut config = record
        .config_snapshot
        .clone()
        .unwrap_or_else(|| current.clone());
    config.restore_credentials(&current);
    config.max_concurrent_jobs = current.max_concurrent_jobs;
    config.max_concurrent_requests = current.max_concurrent_requests;
    let reporter = Reporter::new(job_id);
    reporter.update("queued", 0, 0)?;
    let _slot =
        match crate::scheduler::Slot::acquire("worker", config.max_concurrent_jobs, &control) {
            Ok(slot) => slot,
            Err(error) => {
                jobs::fail(job_id, &format!("{error:#}"))?;
                return Ok(());
            }
        };
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
                jobs::complete(job_id, business_result(text, job_id))?;
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
    "Use LLM2MCP to offload large local code analysis, issue debugging, planning, diff review, repository documentation, and documentation maintenance to the configured LLM. Long-running operations may return a durable background job. Prefer passing file or directory paths plus symptoms/logs instead of reading large files into the primary model first."
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
                        "allow_supplement":{"type":"boolean","default":true,"description":"Allow at most one additional analysis pass after bounded supplemental retrieval"},
                        "task": {"type": "string", "description": "What to analyze and what questions to answer"},
                        "paths": {"type": "array", "items": {"type": "string"}, "description": "Workspace-relative files or directories. Empty triggers lightweight repository discovery before deep file reads."},
                        "include": {"type": "array", "items": {"type": "string"}, "description": "Optional include globs such as src/**/*.rs"},
                        "exclude": {"type": "array", "items": {"type": "string"}, "description": "Optional exclude globs such as tests/** or *.generated.ts"}
                    },
                    "required": ["task"],
                    "additionalProperties": false
                },
                "annotations": {"readOnlyHint": true, "destructiveHint": false}
            },
            {
                "name": "debug_issue",
                "description": "Diagnose a concrete software malfunction from symptoms, optional logs, local source evidence, and optional recent Git changes. Reuses bounded symbol discovery and may run as a background job.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "allow_supplement":{"type":"boolean","default":true,"description":"Allow at most one additional analysis pass after bounded supplemental retrieval"},
                        "issue": {"type": "string", "description": "Required symptom or malfunction description"},
                        "paths": {"type": "array", "items": {"type": "string"}, "description": "Optional workspace-relative files or directories to prioritize"},
                        "logs": {"type": "string", "description": "Optional logs, stack trace, compiler/runtime error, HTTP error, console output, or other observed evidence"},
                        "expected": {"type": "string", "description": "Optional expected behavior"},
                        "actual": {"type": "string", "description": "Optional actual behavior, when useful to distinguish from issue"},
                        "recent_changes": {"type": "boolean", "default": false, "description": "When true, include the current tracked Git diff against HEAD as additional debugging evidence"},
                        "include": {"type": "array", "items": {"type": "string"}, "description": "Optional include globs"},
                        "exclude": {"type": "array", "items": {"type": "string"}, "description": "Optional exclude globs"}
                    },
                    "required": ["issue"],
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
                        "allow_supplement":{"type":"boolean","default":true,"description":"Allow at most one additional analysis pass after bounded supplemental retrieval"},
                        "task": {"type": "string"},
                        "paths": {"type": "array", "items": {"type": "string"}, "description": "Workspace-relative files or directories. Directories use lightweight file/symbol discovery before deep reads."},
                        "include": {"type": "array", "items": {"type": "string"}, "description": "Optional include globs"},
                        "exclude": {"type": "array", "items": {"type": "string"}, "description": "Optional exclude globs"}
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
                        "base_ref": {"type": "string", "default": "HEAD"},
                        "include_untracked": {"type": "boolean", "default": false, "description": "Explicitly include nonignored untracked text files in review"}
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
                        "include": {"type": "array", "items": {"type": "string"}, "description": "Optional include globs"},
                        "exclude": {"type": "array", "items": {"type": "string"}, "description": "Optional exclude globs"},
                        "scan_cursor":{"type":"string","description":"Optional continuation from a previous scan"},
                        "max_chunks":{"type":"integer","minimum":1,"maximum":12,"default":12},
                        "audience": {"type": "string", "default": "developer"},
                        "language": {"type": "string", "enum": ["auto", "english", "simplified_chinese"], "default": "auto"}
                    },
                    "additionalProperties": false
                },
                "annotations": {"readOnlyHint": true, "destructiveHint": false}
            },
            {
                "name": "update_docs",
                "description": "Compare code changes with existing README/docs and return validated exact-fragment edits with original SHA-256 hashes, preserving unseen sections. Never writes files. Defaults to asynchronous execution.",
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
                "name": "result_page",
                "description": "Read a bounded UTF-8 page of a completed job's validated complete result without model calls. Reassemble pages before parsing JSON or applying documents. Cursors are immutable and workspace-bound.",
                "inputSchema": {"type":"object", "properties": {
                    "job_id":{"type":"string"}, "cursor":{"type":"string"},
                    "page_tokens":{"type":"integer","minimum":256,"maximum":8192,"default":1000}
                }, "required":["job_id"], "additionalProperties":false},
                "annotations":{"readOnlyHint":true,"destructiveHint":false}
            },
            {
                "name": "continue_scan",
                "description": "Continue a document_repo scan from its persistent scan_cursor. Uses the original request and accumulated map evidence; changed repositories or layouts require a fresh scan. May return a background job.",
                "inputSchema":{"type":"object", "properties":{"scan_cursor":{"type":"string"}},"required":["scan_cursor"],"additionalProperties":false},
                "annotations":{"readOnlyHint":true,"destructiveHint":false}
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
    let requested_name = params
        .get("name")
        .and_then(Value::as_str)
        .context("missing tool name")?;
    let name = if requested_name == "continue_scan" {
        "document_repo"
    } else {
        requested_name
    };
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    if requested_name == "continue_scan" {
        required_string_arg(&args, "scan_cursor")?;
    }
    if args.get("scan_cursor").is_some() {
        required_string_arg(&args, "scan_cursor")?;
    }
    if args
        .get("allow_supplement")
        .is_some_and(|value| !value.is_boolean())
    {
        bail!("allow_supplement must be a boolean");
    }
    match name {
        "job_status" => return job_status(&args),
        "job_result" => return job_result(&args),
        "result_page" => {
            let job_id = job_id_arg(&args)?;
            let text = jobs::read_full_result(job_id, root)?;
            let budget = args
                .get("page_tokens")
                .map(|value| value.as_u64().context("page_tokens must be an integer"))
                .transpose()?
                .unwrap_or(1000);
            let budget = usize::try_from(budget).context("page_tokens exceeds supported range")?;
            let cursor = args
                .get("cursor")
                .map(|value| value.as_str().context("cursor must be a string"))
                .transpose()?;
            return Ok(tool_result(
                crate::result_pages::page(job_id, &text, cursor, budget)?.to_string(),
                false,
            ));
        }
        "job_cancel" => return job_cancel(&args),
        _ => {}
    }

    let tool = tool_config(config, name)?;
    let _ = crate::cache::maintain(config.cache_max_mib, config.cache_ttl_days, false);
    let execution = effective_execution(name, tool, root, &args);
    if execution == ExecutionMode::Async {
        let record = jobs::create(
            name,
            root,
            args,
            config.job_ttl_hours,
            config.job_poll_interval_ms,
        )?;
        if client_supports_tasks(params) {
            return Ok(jobs::create_task_result(&record));
        }
        return Ok(jobs::fallback_started_result(&record));
    }

    let record = jobs::create_sync(
        name,
        root,
        args.clone(),
        config.job_ttl_hours,
        config.job_poll_interval_ms,
    )?;
    let _heartbeat = jobs::start_heartbeat(&record.id)?;
    let control = crate::control::Control::current().with_job(&record.id);
    control.set_current();
    let reporter = Reporter::new(&record.id);
    let result = crate::scheduler::Slot::acquire("worker", config.max_concurrent_jobs, &control)
        .and_then(|_slot| execute_business_inner(config, root, name, &args, Some(&reporter)));
    match result {
        Ok(text) => {
            let result = business_result(text, &record.id);
            jobs::complete(&record.id, result.clone())?;
            Ok(result)
        }
        Err(error) => {
            if control.check().is_err() {
                jobs::cancel(&record.id)?;
            }
            jobs::fail(&record.id, &format_business_error(&error))?;
            Ok(tool_result(format_business_error(&error), true))
        }
    }
}

fn tool_config<'a>(config: &'a AppConfig, name: &str) -> Result<&'a ToolConfig> {
    match name {
        "analyze" => Ok(&config.tools.analyze),
        "debug_issue" => Ok(&config.tools.debug_issue),
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
    if matches!(name, "analyze" | "plan" | "debug_issue") && paths_arg(args).is_empty() {
        return true;
    }
    if matches!(name, "document_repo" | "update_docs") {
        return true;
    }
    if name == "review_diff" {
        return false;
    }
    if name == "debug_issue"
        && (args
            .get("recent_changes")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || args
                .get("logs")
                .and_then(Value::as_str)
                .is_some_and(|logs| workspace::estimate_tokens(logs) > 3_000))
    {
        return true;
    }
    let paths = paths_arg(args);
    if name == "debug_issue" && paths.is_empty() {
        return true;
    }
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
    jobs::recover_stale().map_err(RpcFailure::internal)?;
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
    jobs::recover_stale()?;
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

fn business_result(text: String, job_id: &str) -> Value {
    let mut result = tool_result(text, false);
    result["_meta"] = json!({"llm2mcp/job_id":job_id, "llm2mcp/full_result_tool":"result_page"});
    result
}

fn full_primary_result(
    raw: &str,
    budget: u32,
    selection: Option<&DeepSelection>,
    reporter: Option<&Reporter>,
) -> Result<String> {
    let Some(reporter) = reporter else {
        return Ok(compact_primary_result(raw, budget, selection));
    };
    let mut value: Value = serde_json::from_str(raw)?;
    if value.get("read_next").is_some() {
        value["read_next"] = json!(validated_read_next_with_limit(
            &value,
            selection,
            usize::MAX,
            usize::MAX
        ));
    }
    reporter.store_full_result(&serde_json::to_string_pretty(&value)?)?;
    let handle = format!("FULL_RESULT job_id={} (result_page)", reporter.job_id());
    let remaining = (budget as usize).saturating_sub(workspace::estimate_tokens(&handle) + 2);
    let compact = compact_primary_result(raw, remaining as u32, selection);
    Ok(workspace::truncate_tokens_strict(
        &format!("{compact}\n\n{handle}"),
        budget as usize,
        "",
    ))
}

fn format_business_error(error: &anyhow::Error) -> String {
    let message = format!("LLM2MCP error: {error:#}");
    if let Some(hint) = jobs::diagnostic_hint(&message) {
        format!("{message}\nDiagnostic hint: {hint}")
    } else {
        message
    }
}

fn execute_business_inner(
    config: &AppConfig,
    root: &Path,
    name: &str,
    args: &Value,
    reporter: Option<&Reporter>,
) -> Result<String> {
    let mut routed = config.for_tool(name)?;
    let system = match name {
        "analyze" => ANALYZE_SYSTEM,
        "debug_issue" => DEBUG_SYSTEM,
        "plan" => PLAN_SYSTEM,
        "review_diff" => REVIEW_SYSTEM,
        "document_repo" => DOC_REDUCE_SYSTEM,
        "update_docs" => UPDATE_DOCS_SYSTEM,
        _ => bail!("unknown tool"),
    };
    let output = routed
        .model_output_limit
        .min((routed.model_input_limit / 4) as u32);
    for tool in [
        &mut routed.tools.analyze,
        &mut routed.tools.debug_issue,
        &mut routed.tools.plan,
        &mut routed.tools.review_diff,
        &mut routed.tools.document_repo,
        &mut routed.tools.update_docs,
    ] {
        tool.max_output_tokens = tool.max_output_tokens.min(output);
    }
    // Continuations reserve the same original request overhead as their first
    // page. A shorter cursor-only call must not silently change Map segmentation.
    let metadata_args = if name == "document_repo" {
        args.get("scan_cursor")
            .and_then(Value::as_str)
            .map(|cursor| crate::scan::load(cursor, root).map(|checkpoint| checkpoint.args))
            .transpose()?
            .unwrap_or_else(|| args.clone())
    } else {
        args.clone()
    };
    let metadata = format!(
        "{}\n{}",
        metadata_args,
        workspace::truncate_tokens_strict(
            &workspace::manifest(root)?,
            1024,
            "[MANIFEST TRUNCATED]"
        )
    );
    let requested_output = match name {
        "analyze" => routed.tools.analyze.max_output_tokens,
        "debug_issue" => routed.tools.debug_issue.max_output_tokens,
        "plan" => routed.tools.plan.max_output_tokens,
        "review_diff" => routed.tools.review_diff.max_output_tokens,
        "document_repo" => routed.tools.document_repo.max_output_tokens,
        _ => routed.tools.update_docs.max_output_tokens,
    };
    let routed = budgeted_config(&routed, system, &metadata, requested_output)?;
    let config = &routed;
    let text = match name {
        "analyze" => analyze(config, root, args, &config.tools.analyze, reporter),
        "debug_issue" => debug_issue(config, root, args, &config.tools.debug_issue, reporter),
        "plan" => plan(config, root, args, &config.tools.plan, reporter),
        "review_diff" => review_diff(config, root, args, &config.tools.review_diff, reporter),
        "document_repo" => document_repo(config, root, args, &config.tools.document_repo, reporter),
        "update_docs" => update_docs(config, root, args, &config.tools.update_docs, reporter),
        _ => bail!("unknown background tool: {name}"),
    }?;
    if let Some(reporter) = reporter
        && jobs::load(reporter.job_id())?.full_result_sha256.is_none()
    {
        reporter.store_full_result(&text)?;
    }
    Ok(text)
}

fn budgeted_config(
    config: &AppConfig,
    system: &str,
    metadata: &str,
    output: u32,
) -> Result<AppConfig> {
    let reserve = workspace::estimate_tokens(&config.system_prompt_prefix)
        + workspace::estimate_tokens(system)
        + workspace::estimate_tokens(metadata)
        + 1024
        + output.min(config.model_output_limit) as usize;
    let available = config.model_input_limit.saturating_sub(reserve);
    if available < 256 || output < 128 {
        bail!(
            "prompt overhead exceeds model context limit {}",
            config.model_input_limit
        );
    }
    let mut bounded = config.clone();
    bounded.max_source_tokens = config.max_source_tokens.min(available);
    bounded.max_file_tokens = config.max_file_tokens.min(available.saturating_sub(128));
    bounded.discovery_index_tokens = config.discovery_index_tokens.min(available);
    Ok(bounded)
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

fn required_string_arg<'a>(args: &'a Value, name: &str) -> Result<&'a str> {
    args.get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .with_context(|| format!("{name} is required"))
}

fn optional_string_arg<'a>(args: &'a Value, name: &str) -> &'a str {
    args.get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
}

fn task_arg(args: &Value) -> Result<&str> {
    required_string_arg(args, "task")
}

fn progress(reporter: Option<&Reporter>, stage: &str, done: u64, total: u64) -> Result<()> {
    crate::control::Control::current().check()?;
    if let Some(reporter) = reporter {
        reporter.update(stage, done, total)?;
    }
    Ok(())
}

fn record_response(
    reporter: Option<&Reporter>,
    response: &llm::ChatResponse,
    config: &AppConfig,
) -> Result<()> {
    if let Some(reporter) = reporter {
        reporter.record_cost(response.prompt_tokens, response.completion_tokens, config)?;
        reporter.record_llm_usage(
            response.prompt_tokens,
            response.completion_tokens,
            &response.diagnostics(),
        )?;
    }
    Ok(())
}

fn chat_detailed_with_reporter(
    config: &AppConfig,
    system: &str,
    user: &str,
    effort: ReasoningEffort,
    max_output_tokens: u32,
    reporter: Option<&Reporter>,
) -> Result<llm::ChatResponse> {
    if let Some(reporter) = reporter {
        reporter.record_llm_attempt()?;
    }
    let started = std::time::Instant::now();
    let result = llm::chat_detailed(config, system, user, effort, max_output_tokens);
    if let Some(reporter) = reporter {
        reporter.record_wait(started.elapsed().as_millis() as u64, &config.model)?;
    }
    let response = result?;
    record_response(reporter, &response, config)?;
    Ok(response)
}

fn chat_with_reporter(
    config: &AppConfig,
    system: &str,
    user: &str,
    effort: ReasoningEffort,
    max_output_tokens: u32,
    reporter: Option<&Reporter>,
) -> Result<String> {
    let attempts = reduce_reasoning_attempts(effort);
    let mut diagnostics = String::new();
    for attempt in 0..3 {
        let prompt = if attempt == 0 {
            user.to_owned()
        } else {
            format!(
                "{user}\n\nRECOVERY: Previous output was incomplete or invalid. Emit the complete requested response, with all required fields and document boundaries. Keep it concise."
            )
        };
        let input = workspace::estimate_tokens(&config.system_prompt_prefix)
            + workspace::estimate_tokens(system)
            + workspace::estimate_tokens(&prompt)
            + 256;
        let ceiling = config
            .model_input_limit
            .saturating_sub(input)
            .min(config.model_output_limit as usize) as u32;
        let budget = max_output_tokens.saturating_mul(1 << attempt).min(ceiling);
        if budget < 128 {
            bail!(
                "estimated input plus output exceeds model context limit; insufficient context for output recovery"
            );
        }
        let response = chat_detailed_with_reporter(
            config,
            system,
            &prompt,
            attempts[attempt.min(attempts.len() - 1)],
            budget,
            reporter,
        )?;
        diagnostics = response.diagnostics();
        if let Some(content) = response.final_text() {
            match validate_model_output(system, user, content) {
                Ok(()) => {
                    return Ok(content.trim().to_owned());
                }
                Err(error) => diagnostics = format!("{diagnostics}; {error}"),
            }
        }
    }
    bail!("model output incomplete or invalid after 3 attempts: {diagnostics}")
}

fn validate_model_output(system: &str, user: &str, content: &str) -> Result<()> {
    if system == DOC_REDUCE_SYSTEM {
        let targets = user
            .split("TARGET DOCUMENTS\n")
            .nth(1)
            .and_then(|text| text.split("\n\n").next())
            .context("missing document targets")?;
        let mut remaining = content.trim();
        for path in targets.lines() {
            let marker = format!("===== DOCUMENT: {path} =====");
            let body = remaining
                .strip_prefix(&marker)
                .context("missing or reordered document boundary")?;
            let (body, rest) = body
                .split_once("===== END DOCUMENT =====")
                .context("unfinished document")?;
            if body.trim().is_empty() {
                bail!("empty document");
            }
            remaining = rest.trim();
        }
        if !remaining.is_empty() {
            bail!("unexpected text after documents");
        }
        return Ok(());
    }
    let (strings, arrays): (&[&str], &[&str]) = if system == ANALYZE_SYSTEM {
        (
            &["conclusion"],
            &["findings", "risks", "actions", "read_next"],
        )
    } else if system == DEBUG_SYSTEM {
        (
            &["diagnosis", "confidence", "root_cause", "intermittency"],
            &[
                "evidence",
                "execution_path",
                "alternatives",
                "verification",
                "fix_area",
                "read_next",
            ],
        )
    } else if system == PLAN_SYSTEM {
        (&["goal"], &["steps", "risks", "tests", "read_next"])
    } else if system == REVIEW_SYSTEM {
        (&["conclusion"], &["findings", "tests", "actions"])
    } else if system == DISCOVERY_SYSTEM {
        (&[], &["symbols", "files"])
    } else if system == UPDATE_DOCS_SYSTEM {
        let value: Value = serde_json::from_str(content).context("invalid document edit JSON")?;
        serde_json::from_value::<crate::doc_edits::Updates>(value)?;
        return Ok(());
    } else {
        return Ok(());
    };
    let value: Value = serde_json::from_str(content).context("invalid response JSON")?;
    if !value.is_object() {
        bail!("response must be an object");
    }
    for key in strings {
        if !value.get(key).is_some_and(Value::is_string) {
            bail!("missing string field {key}");
        }
    }
    if system == DEBUG_SYSTEM
        && !value
            .get("confidence")
            .and_then(Value::as_str)
            .is_some_and(|confidence| ["high", "medium", "low"].contains(&confidence))
    {
        bail!("invalid debug confidence");
    }
    for key in arrays {
        if !value.get(key).is_some_and(Value::is_array) {
            bail!("missing array field {key}");
        }
    }
    for key in arrays {
        for item in value[*key].as_array().expect("array checked") {
            if ["findings", "evidence"].contains(key) {
                if !item.get("text").is_some_and(Value::is_string)
                    || !item
                        .get("severity")
                        .and_then(Value::as_str)
                        .is_some_and(|severity| {
                            ["critical", "high", "medium", "low", "info"].contains(&severity)
                        })
                    || !item
                        .get("evidence")
                        .and_then(Value::as_array)
                        .is_some_and(|refs| refs.iter().all(Value::is_string))
                {
                    bail!("invalid finding in {key}");
                }
            } else if *key == "read_next" {
                if !["path", "symbol", "lines", "reason"]
                    .iter()
                    .all(|field| item.get(field).is_some_and(Value::is_string))
                {
                    bail!("invalid read_next item");
                }
            } else if *key == "symbols" {
                if !item.get("path").is_some_and(Value::is_string)
                    || !item.get("start_line").is_some_and(Value::is_u64)
                    || !item.get("end_line").is_some_and(Value::is_u64)
                {
                    bail!("invalid discovery range");
                }
            } else if !item.is_string() {
                bail!("invalid string list {key}");
            }
        }
    }
    Ok(())
}

fn source_filters(args: &Value) -> (Vec<String>, Vec<String>) {
    (
        string_list_arg(args, "include"),
        string_list_arg(args, "exclude"),
    )
}

#[derive(Debug, Clone)]
struct DeepSelection {
    files: Vec<String>,
    symbols: Vec<workspace::SymbolCandidate>,
    truncated: bool,
    used_llm: bool,
}

impl DeepSelection {
    fn description(&self) -> String {
        let mut lines = self
            .symbols
            .iter()
            .map(|symbol| {
                format!(
                    "{}:{}-{} {}",
                    symbol.path, symbol.start_line, symbol.end_line, symbol.label
                )
            })
            .collect::<Vec<_>>();
        lines.extend(self.files.iter().map(|path| format!("{path} (whole file)")));
        lines.join("\n")
    }
}

fn should_discover(root: &Path, requested: &[String]) -> bool {
    if requested.is_empty() || requested.len() > 1 {
        return true;
    }
    requested.first().is_some_and(|path| {
        let path = root.join(path);
        path.is_dir()
            || path
                .metadata()
                .map(|metadata| metadata.len() > 80_000)
                .unwrap_or(false)
    })
}

fn task_keywords(task: &str) -> Vec<String> {
    crate::search::keywords(task)
}

fn lexical_score(keywords: &[String], text: &str, weight: u32) -> u32 {
    let haystack = text.to_ascii_lowercase();
    keywords
        .iter()
        .filter(|keyword| haystack.contains(keyword.as_str()))
        .map(|_| weight)
        .sum()
}

fn local_discovery_selection(
    task: &str,
    index: &workspace::DiscoveryIndex,
) -> Option<(Vec<workspace::SymbolCandidate>, Vec<String>)> {
    let keywords = task_keywords(task);
    if keywords.is_empty() {
        return None;
    }

    let exact = index
        .symbols
        .iter()
        .filter(|symbol| {
            symbol
                .label
                .split(|ch: char| !ch.is_alphanumeric() && ch != '_')
                .any(|name| name.contains('_') && task.split_whitespace().any(|word| word == name))
        })
        .collect::<Vec<_>>();
    if exact.len() == 1 {
        return Some((vec![exact[0].clone()], Vec::new()));
    }
    let mut symbols = index
        .symbols
        .iter()
        .map(|symbol| {
            let score = lexical_score(&keywords, &symbol.label, 8)
                + lexical_score(&keywords, &symbol.path, 4);
            (score, symbol.clone())
        })
        .filter(|(score, _)| *score > 0)
        .collect::<Vec<_>>();
    symbols.sort_by_key(|item| std::cmp::Reverse(item.0));
    if let Some((top_score, _)) = symbols.first()
        && *top_score >= 16
        && symbols
            .get(1)
            .is_none_or(|(second, _)| *second < *top_score)
    {
        let threshold = (*top_score / 2).max(4);
        let selected = symbols
            .into_iter()
            .filter(|(score, _)| *score >= threshold)
            .take(12)
            .map(|(_, symbol)| symbol)
            .collect::<Vec<_>>();
        if !selected.is_empty() {
            return Some((selected, Vec::new()));
        }
    }

    let mut files = index
        .candidate_files
        .iter()
        .map(|path| (lexical_score(&keywords, path, 5), path.clone()))
        .filter(|(score, _)| *score > 0)
        .collect::<Vec<_>>();
    files.sort_by_key(|item| std::cmp::Reverse(item.0));
    if let Some((top_score, _)) = files.first()
        && *top_score >= 10
        && files.get(1).is_none_or(|(second, _)| *second < *top_score)
    {
        let selected = files
            .into_iter()
            .take(6)
            .map(|(_, path)| path)
            .collect::<Vec<_>>();
        return Some((Vec::new(), selected));
    }
    None
}

fn json_object(response: &str) -> Option<Value> {
    let trimmed = response.trim();
    let slice = trimmed
        .find('{')
        .zip(trimmed.rfind('}'))
        .and_then(|(start, end)| (start <= end).then_some(&trimmed[start..=end]))?;
    serde_json::from_str(slice).ok()
}

fn parse_discovery_selection(
    response: &str,
    index: &workspace::DiscoveryIndex,
) -> (Vec<workspace::SymbolCandidate>, Vec<String>) {
    let Some(value) = json_object(response) else {
        return (Vec::new(), Vec::new());
    };
    let mut symbols = Vec::new();
    if let Some(items) = value.get("symbols").and_then(Value::as_array) {
        for item in items {
            let Some(path) = item.get("path").and_then(Value::as_str) else {
                continue;
            };
            let Some(start_line) = item.get("start_line").and_then(Value::as_u64) else {
                continue;
            };
            let Some(end_line) = item.get("end_line").and_then(Value::as_u64) else {
                continue;
            };
            let candidate = index.symbols.iter().find(|candidate| {
                candidate.path == path
                    && candidate.start_line == start_line as usize
                    && candidate.end_line == end_line as usize
            });
            if let Some(candidate) = candidate
                && !symbols.iter().any(|selected: &workspace::SymbolCandidate| {
                    selected.path == candidate.path && selected.start_line == candidate.start_line
                })
            {
                symbols.push(candidate.clone());
                if symbols.len() >= 12 {
                    break;
                }
            }
        }
    }

    let remaining = 12usize.saturating_sub(symbols.len());
    let mut files = Vec::new();
    if remaining > 0
        && let Some(items) = value.get("files").and_then(Value::as_array)
    {
        for item in items {
            let Some(path) = item.as_str() else {
                continue;
            };
            if index
                .candidate_files
                .iter()
                .any(|candidate| candidate == path)
                && !files.iter().any(|selected| selected == path)
                && !symbols.iter().any(|symbol| symbol.path == path)
            {
                files.push(path.to_owned());
                if files.len() >= remaining {
                    break;
                }
            }
        }
    }
    (symbols, files)
}

fn discover_deep_context(
    config: &AppConfig,
    root: &Path,
    task: &str,
    requested: &[String],
    include: &[String],
    exclude: &[String],
    reporter: Option<&Reporter>,
) -> Result<DeepSelection> {
    if !should_discover(root, requested) {
        return Ok(DeepSelection {
            files: requested.to_vec(),
            symbols: Vec::new(),
            truncated: false,
            used_llm: false,
        });
    }

    let discovery_config = config.with_profile(config.discovery_profile.as_deref())?;
    let discovery_output = 1200
        .min(discovery_config.model_output_limit)
        .min((discovery_config.model_input_limit / 4) as u32);
    let discovery_config =
        budgeted_config(&discovery_config, DISCOVERY_SYSTEM, task, discovery_output)?;
    let index = workspace::discovery_index_for_task(
        root,
        requested,
        &discovery_config,
        include,
        exclude,
        task,
    )?;
    if let Some(reporter) = reporter {
        reporter.record_cache_usage(index.cache_hits as u64, index.cache_misses as u64, 0, 0)?;
    }
    if index.candidate_files.is_empty() {
        bail!("no readable source files matched the requested paths/include/exclude filters");
    }

    if let Some((symbols, files)) = local_discovery_selection(task, &index) {
        return Ok(DeepSelection {
            files,
            symbols,
            truncated: index.truncated,
            used_llm: false,
        });
    }

    if index.candidate_files.len() <= 2 {
        return Ok(DeepSelection {
            files: index.candidate_files,
            symbols: Vec::new(),
            truncated: index.truncated,
            used_llm: false,
        });
    }

    let prompt = format!(
        "TASK\n{task}\n\nCANDIDATE FILE/SYMBOL INDEX\n{}\n\nINDEX_TRUNCATED\n{}\n\nSelect the smallest exact symbol ranges/files needed for a deep second pass. Return JSON only.",
        index.body, index.truncated
    );
    let selection = chat_with_reporter(
        &discovery_config,
        DISCOVERY_SYSTEM,
        &prompt,
        ReasoningEffort::Low,
        discovery_output,
        reporter,
    )?;
    let (mut symbols, mut files) = parse_discovery_selection(&selection, &index);
    if symbols.is_empty() && files.is_empty() {
        files = index.candidate_files.iter().take(8).cloned().collect();
    }
    symbols.truncate(12);
    let remaining = 12usize.saturating_sub(symbols.len());
    files.truncate(remaining);
    Ok(DeepSelection {
        files,
        symbols,
        truncated: index.truncated,
        used_llm: true,
    })
}

fn collect_deep_selection(
    config: &AppConfig,
    root: &Path,
    selection: &DeepSelection,
    include: &[String],
    exclude: &[String],
) -> Result<workspace::CollectedSource> {
    if !selection.symbols.is_empty() && selection.files.is_empty() {
        workspace::collect_symbol_context(root, &selection.symbols, config)
    } else {
        let mut sources = if selection.files.is_empty() {
            workspace::collect_symbol_context(root, &selection.symbols, config)?
        } else {
            workspace::collect_filtered(root, &selection.files, config, include, exclude)?
        };
        if !selection.symbols.is_empty() {
            let symbol_sources =
                workspace::collect_symbol_context(root, &selection.symbols, config)?;
            let remaining = config
                .max_source_tokens
                .saturating_sub(workspace::estimate_tokens(&sources.body));
            if remaining > 128 {
                let (extra, clipped) = workspace::truncate_tokens(
                    &symbol_sources.body,
                    remaining,
                    "\n[ADDITIONAL SYMBOL CONTEXT TRUNCATED]\n",
                );
                sources.body.push_str(&extra);
                sources.truncated |= clipped || symbol_sources.truncated;
                sources.evidence_cache_hits = sources
                    .evidence_cache_hits
                    .saturating_add(symbol_sources.evidence_cache_hits);
                sources.evidence_cache_misses = sources
                    .evidence_cache_misses
                    .saturating_add(symbol_sources.evidence_cache_misses);
                for path in symbol_sources.included_files {
                    if !sources.included_files.contains(&path) {
                        sources.included_files.push(path);
                    }
                }
            }
        }
        Ok(sources)
    }
}

fn validated_read_next(value: &Value, selection: Option<&DeepSelection>) -> Vec<String> {
    validated_read_next_with_limit(value, selection, 6, 4)
}

fn validated_read_next_with_limit(
    value: &Value,
    selection: Option<&DeepSelection>,
    limit: usize,
    file_limit: usize,
) -> Vec<String> {
    let Some(selection) = selection else {
        return Vec::new();
    };
    let mut output = Vec::new();
    if let Some(items) = value.get("read_next").and_then(Value::as_array) {
        for item in items {
            let Some(path) = item.get("path").and_then(Value::as_str) else {
                continue;
            };
            let lines = item
                .get("lines")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let parsed_lines = lines.split_once('-').and_then(|(start, end)| {
                Some((
                    start.trim().parse::<usize>().ok()?,
                    end.trim().parse::<usize>().ok()?,
                ))
            });
            let Some((start_line, end_line)) = parsed_lines else {
                continue;
            };
            let Some(candidate) = selection.symbols.iter().find(|candidate| {
                candidate.path == path
                    && candidate.start_line == start_line
                    && candidate.end_line == end_line
            }) else {
                continue;
            };
            let reason = item
                .get("reason")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty());
            let suffix = reason
                .map(|value| format!(" — {value}"))
                .unwrap_or_default();
            output.push(format!(
                "- {}:{}-{} — {}{}",
                candidate.path, candidate.start_line, candidate.end_line, candidate.label, suffix
            ));
            if output.len() >= limit {
                break;
            }
        }
    }

    if output.is_empty() {
        output.extend(selection.symbols.iter().take(limit).map(|candidate| {
            format!(
                "- {}:{}-{} — {}",
                candidate.path, candidate.start_line, candidate.end_line, candidate.label
            )
        }));
    }
    if output.is_empty() {
        output.extend(
            selection
                .files
                .iter()
                .take(file_limit)
                .map(|path| format!("- {path}")),
        );
    }
    output
}

fn compact_primary_result(raw: &str, max_tokens: u32, selection: Option<&DeepSelection>) -> String {
    let budget = max_tokens as usize;
    if budget == 0 {
        return String::new();
    }
    let Some(value) = json_object(raw) else {
        let mut fallback = String::new();
        let read_next = selection
            .map(|selection| {
                let placeholder = json!({});
                validated_read_next(&placeholder, Some(selection))
            })
            .unwrap_or_default();
        if !read_next.is_empty() {
            fallback.push_str("READ_NEXT\n");
            fallback.push_str(&read_next.join("\n"));
            fallback.push_str("\n\nRESULT\n");
        }
        fallback.push_str(raw.trim());
        return workspace::truncate_tokens_strict(
            &fallback,
            budget,
            "\n[PRIMARY RESULT TRUNCATED]\n",
        );
    };

    crate::primary_result::compact(&value, &validated_read_next(&value, selection), budget)
}

fn analyze(
    config: &AppConfig,
    root: &Path,
    args: &Value,
    tool: &ToolConfig,
    reporter: Option<&Reporter>,
) -> Result<String> {
    progress(reporter, "building discovery index", 0, 3)?;
    let task = task_arg(args)?;
    let requested = paths_arg(args);
    let (include, exclude) = source_filters(args);
    let mut selection =
        discover_deep_context(config, root, task, &requested, &include, &exclude, reporter)?;
    progress(reporter, "collecting selected context", 1, 3)?;
    let mut sources = collect_deep_selection(config, root, &selection, &include, &exclude)?;
    if let Some(reporter) = reporter {
        reporter.record_context(
            workspace::estimate_tokens(&sources.body),
            sources.included_files.len(),
            sources.truncated,
        )?;
        reporter.record_cache_usage(
            0,
            0,
            sources.evidence_cache_hits as u64,
            sources.evidence_cache_misses as u64,
        )?;
    }
    if sources.included_files.is_empty() {
        bail!("no readable source files found after discovery and filtering");
    }
    progress(reporter, "analyzing", 2, 3)?;
    let prompt = format!(
        "TASK\n{task}\n\nREPOSITORY MANIFEST\n{}\n\nDISCOVERY_SELECTED_CONTEXT\n{}\n\nDISCOVERY_USED_LLM\n{}\n\nDISCOVERY_INDEX_TRUNCATED\n{}\n\nFILES INCLUDED\n{}\n\nESTIMATED_SOURCE_TOKENS\n{}\n\nSOURCE MATERIAL\n{}\n\nSOURCE_TRUNCATED\n{}\n\nReturn the JSON shape required by the system prompt. For read_next, copy only exact path/symbol/line ranges shown in DISCOVERY_SELECTED_CONTEXT.",
        sources.manifest,
        selection.description(),
        selection.used_llm,
        selection.truncated,
        sources.included_files.join("\n"),
        workspace::estimate_tokens(&sources.body),
        sources.body,
        sources.truncated,
    );
    let context = AnalysisContext {
        config,
        root,
        system: ANALYZE_SYSTEM,
        tool,
        filters: (&include, &exclude),
        allow: args
            .get("allow_supplement")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        reporter,
    };
    let result = analyze_with_supplement(&context, &prompt, &mut selection, &mut sources)?;
    let result = crate::evidence::verify(
        serde_json::from_str(&result)?,
        &crate::evidence::Catalog::new(&sources.body, "", ""),
    )
    .to_string();
    progress(reporter, "finishing", 3, 3)?;
    full_primary_result(
        &result,
        tool.primary_return_or(800),
        Some(&selection),
        reporter,
    )
}

fn debug_issue(
    config: &AppConfig,
    root: &Path,
    args: &Value,
    tool: &ToolConfig,
    reporter: Option<&Reporter>,
) -> Result<String> {
    progress(reporter, "preparing debug evidence", 0, 4)?;
    let issue = required_string_arg(args, "issue")?;
    let expected = optional_string_arg(args, "expected");
    let actual = optional_string_arg(args, "actual");
    let logs = optional_string_arg(args, "logs");
    let recent_changes = args
        .get("recent_changes")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let requested = paths_arg(args);
    let (include, exclude) = source_filters(args);

    // Reserve part of the configured source budget for runtime evidence so
    // logs/diffs do not silently make debug requests much larger than normal.
    let mut debug_context_config = config.clone();
    debug_context_config.max_source_tokens = (config.max_source_tokens / 2).max(128);
    let auxiliary_budget = config
        .max_source_tokens
        .saturating_sub(debug_context_config.max_source_tokens)
        .max(128);
    let logs_budget = (auxiliary_budget / 2).clamp(64, 8_000);
    let diff_budget = auxiliary_budget
        .saturating_sub(logs_budget)
        .clamp(64, 12_000);

    let logs_for_prompt =
        workspace::truncate_tokens_strict(logs, logs_budget, "\n[DEBUG LOGS TRUNCATED]\n");
    let logs_for_discovery = workspace::truncate_tokens_strict(
        logs,
        logs_budget.min(1_500),
        "\n[LOGS TRUNCATED FOR DISCOVERY]\n",
    );

    let recent_change_evidence = if recent_changes {
        match workspace::git_diff(root, "HEAD", diff_budget) {
            Ok((status, diff, truncated)) => format!(
                "GIT STATUS\n{status}\n\nDIFF AGAINST HEAD\n{diff}\n\nDIFF_TRUNCATED\n{truncated}"
            ),
            Err(error) => format!("Recent Git changes requested but unavailable: {error:#}"),
        }
    } else {
        "Not requested.".to_owned()
    };
    let recent_changes_for_discovery = workspace::truncate_tokens_strict(
        &recent_change_evidence,
        1_500,
        "\n[RECENT CHANGES TRUNCATED FOR DISCOVERY]\n",
    );
    let discovery_task = format!(
        "ISSUE\n{issue}\n\nEXPECTED\n{expected}\n\nACTUAL\n{actual}\n\nOBSERVED LOGS\n{logs_for_discovery}\n\nRECENT CHANGES\n{recent_changes_for_discovery}"
    );

    progress(reporter, "building debug discovery index", 1, 4)?;
    let mut selection = discover_deep_context(
        &debug_context_config,
        root,
        &discovery_task,
        &requested,
        &include,
        &exclude,
        reporter,
    )?;
    progress(reporter, "collecting debug context", 2, 4)?;
    let mut sources =
        collect_deep_selection(&debug_context_config, root, &selection, &include, &exclude)?;
    if let Some(reporter) = reporter {
        reporter.record_context(
            workspace::estimate_tokens(&sources.body),
            sources.included_files.len(),
            sources.truncated,
        )?;
        reporter.record_cache_usage(
            0,
            0,
            sources.evidence_cache_hits as u64,
            sources.evidence_cache_misses as u64,
        )?;
    }
    if sources.included_files.is_empty() {
        bail!("no readable source files found after debug discovery and filtering");
    }

    progress(reporter, "diagnosing issue", 3, 4)?;
    let prompt = format!(
        "ISSUE\n{issue}\n\nEXPECTED BEHAVIOR\n{expected}\n\nACTUAL BEHAVIOR\n{actual}\n\nOBSERVED LOGS / STACK TRACE / ERRORS\n{logs_for_prompt}\n\nRECENT GIT CHANGES\n{recent_change_evidence}\n\nREPOSITORY MANIFEST\n{}\n\nDISCOVERY_SELECTED_CONTEXT\n{}\n\nDISCOVERY_USED_LLM\n{}\n\nDISCOVERY_INDEX_TRUNCATED\n{}\n\nFILES INCLUDED\n{}\n\nESTIMATED_SOURCE_TOKENS\n{}\n\nSOURCE MATERIAL\n{}\n\nSOURCE_TRUNCATED\n{}\n\nDiagnose the malfunction using only supplied evidence. Return the JSON shape required by the system prompt. For read_next, copy only exact path/symbol/line ranges shown in DISCOVERY_SELECTED_CONTEXT.",
        sources.manifest,
        selection.description(),
        selection.used_llm,
        selection.truncated,
        sources.included_files.join("\n"),
        workspace::estimate_tokens(&sources.body),
        sources.body,
        sources.truncated,
    );
    let context = AnalysisContext {
        config: &debug_context_config,
        root,
        system: DEBUG_SYSTEM,
        tool,
        filters: (&include, &exclude),
        allow: args
            .get("allow_supplement")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        reporter,
    };
    let result = analyze_with_supplement(&context, &prompt, &mut selection, &mut sources)?;
    let result = crate::evidence::verify(
        serde_json::from_str(&result)?,
        &crate::evidence::Catalog::new(&sources.body, &logs_for_prompt, &recent_change_evidence),
    )
    .to_string();
    progress(reporter, "finishing", 4, 4)?;
    full_primary_result(
        &result,
        tool.primary_return_or(1_400),
        Some(&selection),
        reporter,
    )
}

fn plan(
    config: &AppConfig,
    root: &Path,
    args: &Value,
    tool: &ToolConfig,
    reporter: Option<&Reporter>,
) -> Result<String> {
    progress(reporter, "building discovery index", 0, 3)?;
    let task = task_arg(args)?;
    let requested = paths_arg(args);
    let (include, exclude) = source_filters(args);
    let mut selection =
        discover_deep_context(config, root, task, &requested, &include, &exclude, reporter)?;
    progress(reporter, "collecting selected context", 1, 3)?;
    let mut sources = collect_deep_selection(config, root, &selection, &include, &exclude)?;
    if let Some(reporter) = reporter {
        reporter.record_context(
            workspace::estimate_tokens(&sources.body),
            sources.included_files.len(),
            sources.truncated,
        )?;
        reporter.record_cache_usage(
            0,
            0,
            sources.evidence_cache_hits as u64,
            sources.evidence_cache_misses as u64,
        )?;
    }
    if sources.included_files.is_empty() {
        bail!("no readable source files found after discovery and filtering");
    }
    progress(reporter, "planning", 2, 3)?;
    let prompt = format!(
        "TASK\n{task}\n\nREPOSITORY MANIFEST\n{}\n\nDISCOVERY_SELECTED_CONTEXT\n{}\n\nDISCOVERY_USED_LLM\n{}\n\nDISCOVERY_INDEX_TRUNCATED\n{}\n\nFILES INCLUDED\n{}\n\nESTIMATED_SOURCE_TOKENS\n{}\n\nSOURCE MATERIAL\n{}\n\nSOURCE_TRUNCATED\n{}\n\nReturn the JSON shape required by the system prompt. For read_next, copy only exact path/symbol/line ranges shown in DISCOVERY_SELECTED_CONTEXT.",
        sources.manifest,
        selection.description(),
        selection.used_llm,
        selection.truncated,
        sources.included_files.join("\n"),
        workspace::estimate_tokens(&sources.body),
        sources.body,
        sources.truncated,
    );
    let context = AnalysisContext {
        config,
        root,
        system: PLAN_SYSTEM,
        tool,
        filters: (&include, &exclude),
        allow: args
            .get("allow_supplement")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        reporter,
    };
    let result = analyze_with_supplement(&context, &prompt, &mut selection, &mut sources)?;
    progress(reporter, "finishing", 3, 3)?;
    full_primary_result(
        &result,
        tool.primary_return_or(1_200),
        Some(&selection),
        reporter,
    )
}

fn compact_review_result(
    value: &Value,
    max_tokens: u32,
    batches: usize,
    include_untracked: bool,
    omitted: &[String],
) -> String {
    let budget = max_tokens as usize;
    let coverage = format!(
        "REVIEW COVERAGE: {batches} batches reviewed; include_untracked={include_untracked}; omitted={}\n{}",
        omitted.len(),
        omitted.join("\n")
    );
    let prefix = workspace::truncate_tokens_strict(
        coverage.trim(),
        (budget / 3).min(256),
        "\n[OMITTED DETAILS TRUNCATED]",
    );
    let remaining = budget.saturating_sub(workspace::estimate_tokens(&prefix) + 2);
    let result = compact_primary_result(&value.to_string(), remaining as u32, None);
    workspace::truncate_tokens_strict(
        &format!("{prefix}\n\n{result}"),
        budget,
        "\n[PRIMARY RESULT TRUNCATED]\n",
    )
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
    let include_untracked = args
        .get("include_untracked")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let (batches, omitted) =
        workspace::review_batches(root, base_ref, include_untracked, config.max_source_tokens)?;
    if batches.is_empty() && omitted.is_empty() {
        return Ok("No eligible git diff found for review.".into());
    }
    let mut combined =
        json!({"conclusion":"Batch review completed", "findings":[], "tests":[], "actions":[]});
    for (index, batch) in batches.iter().enumerate() {
        progress(
            reporter,
            "reviewing diff batches",
            index as u64,
            batches.len() as u64,
        )?;
        let prompt = format!(
            "REVIEW TASK\n{task}\n\nDIFF AGAINST {base_ref}\nFILE {}\nBATCH {}/{}\n{}\n\nSURROUNDING CURRENT SOURCE\n{}\n\nReview only supplied evidence; an individual batch may contain part of a large diff. Return the required review JSON.",
            batch.path,
            index + 1,
            batches.len(),
            batch.diff,
            batch.context
        );
        let result = chat_with_reporter(
            config,
            REVIEW_SYSTEM,
            &prompt,
            tool.reasoning,
            tool.max_output_tokens,
            reporter,
        )?;
        let value = crate::evidence::verify(
            serde_json::from_str(&result)?,
            &crate::evidence::Catalog::new(&batch.context, "", &batch.diff),
        );
        for key in ["findings", "tests", "actions"] {
            for item in value[key].as_array().context("missing review array")? {
                let output = combined[key]
                    .as_array_mut()
                    .expect("review arrays initialized");
                if !output.contains(item) {
                    output.push(item.clone());
                }
            }
        }
    }
    combined["findings"]
        .as_array_mut()
        .expect("findings initialized")
        .sort_by_key(|item| match item["severity"].as_str().unwrap_or("") {
            "critical" => 0,
            "high" => 1,
            "medium" => 2,
            "low" => 3,
            _ => 4,
        });
    if let Some(reporter) = reporter {
        reporter.record_context(
            batches
                .iter()
                .map(|b| {
                    workspace::estimate_tokens(&b.diff) + workspace::estimate_tokens(&b.context)
                })
                .sum(),
            batches.len(),
            !omitted.is_empty(),
        )?;
    }
    progress(
        reporter,
        "finishing",
        batches.len() as u64,
        batches.len() as u64,
    )?;
    let budget = tool.primary_return_or(1000);
    let handle = if let Some(reporter) = reporter {
        let full = json!({"review":combined, "coverage":{"batches":batches.len(), "include_untracked":include_untracked, "omitted":omitted}});
        reporter.store_full_result(&serde_json::to_string_pretty(&full)?)?;
        format!("\n\nFULL_RESULT job_id={} (result_page)", reporter.job_id())
    } else {
        String::new()
    };
    let remaining = budget.saturating_sub(workspace::estimate_tokens(&handle) as u32);
    let compact = compact_review_result(
        &combined,
        remaining,
        batches.len(),
        include_untracked,
        &omitted,
    );
    Ok(workspace::truncate_tokens_strict(
        &format!("{compact}{handle}"),
        budget as usize,
        "",
    ))
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
    reporter: Option<&Reporter>,
) -> Result<String> {
    let prompt = format!(
        "CHUNK {}/{}\n\nFILES\n{}\n\nSOURCE\n{}\n\nExtract repository-map evidence for later documentation synthesis. Focus on responsibilities, entry points, important symbols, flows, APIs, configuration, build/deploy behavior, integrations, and constraints. Cite file paths and symbols. Keep the result concise; this summary will be combined with other chunks later.\n",
        index + 1,
        total_chunks,
        chunk.included_files.join("\n"),
        chunk.body,
    );
    chat_with_reporter(
        config,
        DOC_MAP_SYSTEM,
        &prompt,
        ReasoningEffort::Low,
        map_tokens,
        reporter,
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
    const MAX_CHUNK_TOKENS: usize = 40_000;

    progress(reporter, "scanning repository", 0, 1)?;
    let previous = args
        .get("scan_cursor")
        .and_then(Value::as_str)
        .map(|cursor| crate::scan::load(cursor, root))
        .transpose()?;
    if let Some(previous) = &previous {
        for (key, value) in args.as_object().context("invalid scan arguments")? {
            if key != "scan_cursor" && previous.args.get(key) != Some(value) {
                bail!("continuation options changed; start a new scan");
            }
        }
    }
    let args = previous
        .as_ref()
        .map(|previous| &previous.args)
        .unwrap_or(args);
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
    let map_config = config.with_profile(config.map_profile.as_deref())?;
    let map_tokens = (tool.max_output_tokens / 10)
        .clamp(128, 1200)
        .min(map_config.model_output_limit)
        .min((map_config.model_input_limit / 4) as u32);
    let map_config = budgeted_config(&map_config, DOC_MAP_SYSTEM, &args.to_string(), map_tokens)?;
    let empty_chunk = workspace::CollectedSource {
        manifest: String::new(),
        body: String::new(),
        included_files: Vec::new(),
        truncated: false,
        evidence_cache_hits: 0,
        evidence_cache_misses: 0,
    };
    let layout = crate::result_pages::digest(&format!(
        "{}:{}:{}:{}",
        doc_cache::map_key(&map_config, map_tokens, &empty_chunk),
        map_config.max_source_tokens,
        map_config.max_file_tokens,
        args
    ));
    if let Some(previous) = &previous
        && previous.layout != layout
    {
        bail!("scan model/profile/budget layout changed; start a new scan");
    }
    let page = crate::scan::collect(
        root,
        args,
        &map_config,
        map_config.max_source_tokens.min(MAX_CHUNK_TOKENS),
        previous.as_ref(),
    )?;
    let chunks = &page.chunks;
    if chunks.is_empty()
        && previous
            .as_ref()
            .is_none_or(|previous| previous.summary_keys.is_empty())
    {
        bail!("no readable source files found for documentation");
    }
    let coverage_note = format!(
        "{}\nPending scan continuation: {}",
        page.coverage.summary(),
        page.next.is_some()
    );

    // Repository-map summaries are evidence extraction, not final prose. Keeping
    // them compact dramatically reduces both map time and final Reduce context.

    if let Some(reporter) = reporter {
        reporter.record_context(
            chunks
                .iter()
                .map(|chunk| workspace::estimate_tokens(&chunk.body))
                .sum(),
            chunks
                .iter()
                .flat_map(|chunk| chunk.included_files.iter())
                .collect::<std::collections::HashSet<_>>()
                .len(),
            page.next.is_some() || !page.coverage.omissions.is_empty(),
        )?;
    }
    let total_chunks = chunks.len();
    let total_steps = total_chunks as u64 + 1;
    let mut scan_truncated = page.next.is_some() || !page.coverage.omissions.is_empty();
    let manifest = chunks
        .first()
        .map(|chunk| chunk.manifest.clone())
        .unwrap_or(workspace::manifest(root)?);
    let concurrency = config
        .document_map_concurrency
        .clamp(1, 8)
        .min(total_chunks.max(1));

    let mut summaries: Vec<Option<String>> = vec![None; total_chunks];
    let mut pending = Vec::new();
    let mut completed = 0usize;

    for (index, chunk) in chunks.iter().enumerate() {
        let key = doc_cache::map_key(&map_config, map_tokens, chunk);
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
                let map_config = &map_config;
                let control = crate::control::Control::current();
                handles.push(scope.spawn(move || -> Result<(usize, String)> {
                    control.set_current();
                    eprintln!(
                        "LLM2MCP document_repo mapping chunk {}/{} ({} files)",
                        index + 1,
                        total_chunks,
                        chunk.included_files.len()
                    );
                    let summary = map_repository_chunk(
                        map_config,
                        chunk,
                        index,
                        total_chunks,
                        map_tokens,
                        reporter,
                    )?;
                    doc_cache::store_summary(&key, &summary)?;
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

    crate::scan::validate_snapshot(root, args, &page.snapshot)?;
    let mut summary_keys = previous
        .as_ref()
        .map(|previous| previous.summary_keys.clone())
        .unwrap_or_default();
    let mut accumulated = summary_keys
        .iter()
        .map(|key| {
            doc_cache::load_summary(key)?.context("scan map cache expired; start a new scan")
        })
        .collect::<Result<Vec<_>>>()?;
    for (index, summary) in summaries.into_iter().enumerate() {
        accumulated.push(summary.with_context(|| format!("missing map summary {index}"))?);
        summary_keys.push(doc_cache::map_key(&map_config, map_tokens, &chunks[index]));
    }
    let summaries_budget = config.max_source_tokens;
    let prefix = workspace::truncate_tokens_strict(
        &coverage_note,
        1024.min(summaries_budget / 4),
        "[COVERAGE TRUNCATED]",
    );
    let per_summary = summaries_budget
        .saturating_sub(workspace::estimate_tokens(&prefix) + accumulated.len() * 2)
        / accumulated.len().max(1);
    let mut summaries_truncated = accumulated.iter().enumerate().any(|(index, summary)| {
        workspace::estimate_tokens(&format!("MAP {}: {summary}", index + 1)) > per_summary
    });
    scan_truncated |= summaries_truncated;
    let complete_summaries = format!(
        "{prefix}\n\n{}",
        accumulated
            .iter()
            .enumerate()
            .map(|(index, summary)| {
                workspace::truncate_tokens_strict(
                    &format!("MAP {}: {summary}", index + 1),
                    per_summary,
                    "[MAP SUMMARY TRUNCATED FOR SYNTHESIS]",
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    );
    let summaries_text = workspace::truncate_tokens_strict(
        &complete_summaries,
        summaries_budget,
        "[MAP SUMMARIES TRUNCATED]",
    );
    summaries_truncated |= summaries_text != complete_summaries;
    scan_truncated |= summaries_truncated;
    if let Some(reporter) = reporter {
        reporter.mark_context_truncated(scan_truncated)?;
    }
    let base_prompt = format!(
        "DOCUMENT TYPE\n{document_type}\n\nTARGET DOCUMENTS\n{}\n\nAUDIENCE\n{audience}\n\nOUTPUT LANGUAGE\n{language}\n\nREPOSITORY MANIFEST\n{manifest}\n\nREPOSITORY MAP SUMMARIES\n{}\n\nSCAN_TRUNCATED\n{scan_truncated}\n\nSynthesize the requested documentation. For full mode, return every target document as a complete Markdown document. For a single document type, return exactly that target document. Ground claims in the repository evidence and mark unknowns instead of guessing. Emit the final Markdown directly; do not spend the output budget restating your analysis.",
        document_targets(document_type),
        summaries_text,
    );

    progress(
        reporter,
        "synthesizing documentation",
        total_chunks as u64,
        total_steps,
    )?;
    let content = chat_with_reporter(
        config,
        DOC_REDUCE_SYSTEM,
        &base_prompt,
        tool.reasoning,
        tool.max_output_tokens,
        reporter,
    )?;
    crate::scan::validate_snapshot(root, args, &page.snapshot)?;
    let cursor = crate::scan::checkpoint(root, args.clone(), &page, layout, summary_keys)?;
    let continuation = json!({"scan_cursor":cursor, "complete":page.next.is_none(), "files_complete":page.coverage.complete_files, "eligible_files":page.coverage.total_files, "segments":page.coverage.segments, "synthesis_truncated":summaries_truncated, "omitted_count":page.coverage.omissions.len()});
    progress(reporter, "finishing", total_steps, total_steps)?;
    Ok(format!(
        "{coverage_note}\nSCAN_CONTINUATION\n{continuation}\n\n{content}"
    ))
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

    let per_side_tokens = (config.max_source_tokens / 2).max(128);
    let (diff, diff_truncated, comparison) = if target_ref.eq_ignore_ascii_case("WORKTREE") {
        let (_status, diff, truncated) = workspace::git_diff(root, base_ref, per_side_tokens)
            .context("update_docs requires a Git repository with a valid base_ref")?;
        (diff, truncated, format!("{base_ref} -> WORKTREE"))
    } else {
        let (diff, truncated) =
            workspace::git_diff_refs(root, base_ref, target_ref, per_side_tokens)
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
    let preview_budget = (per_side_tokens / doc_paths.len().clamp(1, 64))
        .min(config.max_file_tokens)
        .max(64);
    let snapshots = doc_paths
        .iter()
        .take(64)
        .map(|path| workspace::document_snapshot(root, path, preview_budget, &diff))
        .collect::<Result<Vec<_>>>()?;
    let doc_list = snapshots
        .iter()
        .map(|doc| doc.path.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let doc_body = snapshots
        .iter()
        .map(|doc| {
            serde_json::to_string(&json!({
                "path":doc.path,"original_sha256":doc.hash,
                "preview_fragments":doc.fragments,"preview_truncated":doc.preview_truncated
            }))
        })
        .collect::<std::result::Result<Vec<_>, _>>()?
        .join("\n\n");
    let docs_truncated =
        snapshots.len() < doc_paths.len() || snapshots.iter().any(|doc| doc.preview_truncated);

    if let Some(reporter) = reporter {
        reporter.record_context(
            workspace::estimate_tokens(&diff).saturating_add(workspace::estimate_tokens(&doc_body)),
            doc_paths.len(),
            diff_truncated || docs_truncated,
        )?;
    }
    progress(reporter, "updating documentation", 2, 3)?;
    let prompt = format!(
        "COMPARISON\n{comparison}\n\nOUTPUT LANGUAGE\n{language}\n\nCODE DIFF\n{diff}\n\nDIFF_TRUNCATED\n{diff_truncated}\n\nEXISTING DOCUMENTS\n{doc_list}\n\nDOCUMENT SNAPSHOTS\n{doc_body}\n\nDOCS_TRUNCATED\n{docs_truncated}\n\nReturn exact local edits only for supplied source fragments affected by the code changes. Preserve every unseen original section. Return JSON in the system schema.",
    );
    let result = chat_with_reporter(
        config,
        UPDATE_DOCS_SYSTEM,
        &prompt,
        tool.reasoning,
        tool.max_output_tokens,
        reporter,
    )?;
    let updates = crate::doc_edits::validate(
        json_object(&result).context("document update returned invalid JSON")?,
        &snapshots,
        root,
    )?;
    progress(reporter, "finishing", 3, 3)?;
    Ok(serde_json::to_string_pretty(&json!({
        "format":"llm2mcp-document-edits-v1", "edits":updates.edits,
        "new_documents":updates.new_documents, "source_preview_truncated":docs_truncated,
        "diff_truncated":diff_truncated,
        "omitted_documents":doc_paths.iter().skip(64).collect::<Vec<_>>(),
        "apply_instructions":"Before applying, verify each file's current SHA-256 equals original_sha256, then replace each old_text exactly once; preserve all other text. Reject stale files and overlapping edits. No files have been written."
    }))?)
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
        assert_eq!(tools.len(), 11);
        assert!(tools.iter().any(|tool| tool["name"] == "job_status"));
        assert!(tools.iter().any(|tool| tool["name"] == "document_repo"));
        let debug = tools
            .iter()
            .find(|tool| tool["name"] == "debug_issue")
            .expect("debug_issue tool");
        assert_eq!(debug["inputSchema"]["required"], json!(["issue"]));
        assert_eq!(
            debug["inputSchema"]["properties"]["recent_changes"]["default"],
            false
        );
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
    fn discovery_selection_accepts_only_known_candidate_ranges_and_paths() {
        let index = workspace::DiscoveryIndex {
            body: String::new(),
            candidate_files: vec![
                "src/main.rs".to_owned(),
                "src/workspace.rs".to_owned(),
                "README.md".to_owned(),
            ],
            symbols: vec![workspace::SymbolCandidate {
                path: "src/workspace.rs".to_owned(),
                label: "pub fn collect_symbol_context(...)".to_owned(),
                start_line: 10,
                end_line: 40,
            }],
            truncated: false,
            cache_hits: 0,
            cache_misses: 0,
        };
        let response = r#"```json
{"symbols":[{"path":"src/workspace.rs","start_line":10,"end_line":40},{"path":"invented.rs","start_line":1,"end_line":2}],"files":["src/main.rs","invented.rs"]}
```"#;
        let (symbols, files) = parse_discovery_selection(response, &index);
        assert_eq!(symbols, index.symbols);
        assert_eq!(files, vec!["src/main.rs".to_owned()]);
    }

    #[test]
    fn local_discovery_scoring_prefers_matching_symbol_without_rerank() {
        let index = workspace::DiscoveryIndex {
            body: String::new(),
            candidate_files: vec!["src/jobs.rs".to_owned(), "src/gui.rs".to_owned()],
            symbols: vec![
                workspace::SymbolCandidate {
                    path: "src/jobs.rs".to_owned(),
                    label: "fn spawn_worker(job_id: &str)".to_owned(),
                    start_line: 400,
                    end_line: 445,
                },
                workspace::SymbolCandidate {
                    path: "src/gui.rs".to_owned(),
                    label: "fn format_duration(ms: u64)".to_owned(),
                    start_line: 80,
                    end_line: 95,
                },
            ],
            truncated: false,
            cache_hits: 0,
            cache_misses: 0,
        };
        let (symbols, files) = local_discovery_selection(
            "diagnose spawn_worker worker executable startup failure",
            &index,
        )
        .expect("strong lexical match should avoid LLM rerank");
        assert!(files.is_empty());
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].path, "src/jobs.rs");
    }

    #[test]
    fn output_validation_rejects_nested_schema_errors_and_unfinished_documents() {
        let value = json!({"conclusion":"ok","findings":[{"text":"bug","severity":"high","evidence":"not an array"}],"risks":[],"actions":[],"read_next":[]});
        assert!(validate_model_output(ANALYZE_SYSTEM, "", &value.to_string()).is_err());
        let prompt = "TARGET DOCUMENTS\ndocs/PROJECT_OVERVIEW.md\n\n";
        assert!(
            validate_model_output(
                DOC_REDUCE_SYSTEM,
                prompt,
                "===== DOCUMENT: docs/PROJECT_OVERVIEW.md =====\n# Partial"
            )
            .is_err()
        );
        assert!(validate_model_output(DOC_REDUCE_SYSTEM, prompt, "===== DOCUMENT: docs/PROJECT_OVERVIEW.md =====\n# Complete\n===== END DOCUMENT =====").is_ok());
    }

    #[test]
    fn ambiguous_single_word_discovery_requires_reranking() {
        let index = workspace::DiscoveryIndex {
            body: String::new(),
            candidate_files: vec!["a.rs".into(), "b.rs".into(), "c.rs".into()],
            symbols: vec![
                workspace::SymbolCandidate {
                    path: "a.rs".into(),
                    label: "fn login()".into(),
                    start_line: 1,
                    end_line: 2,
                },
                workspace::SymbolCandidate {
                    path: "b.rs".into(),
                    label: "fn login()".into(),
                    start_line: 1,
                    end_line: 2,
                },
            ],
            truncated: false,
            cache_hits: 0,
            cache_misses: 0,
        };
        assert!(local_discovery_selection("login", &index).is_none());
    }

    #[test]
    fn debug_issue_auto_mode_escalates_large_runtime_evidence() {
        let root = Path::new(".");
        let large_logs = "failure ".repeat(4_000);
        assert!(auto_should_async(
            "debug_issue",
            root,
            &json!({"issue": "intermittent failure", "logs": large_logs})
        ));
        assert!(auto_should_async(
            "debug_issue",
            root,
            &json!({"issue": "regression", "recent_changes": true})
        ));
        assert!(auto_should_async(
            "debug_issue",
            root,
            &json!({"issue": "unknown failure with no path hints"})
        ));
    }

    #[test]
    fn review_return_budget_includes_coverage_and_long_omission_lists() {
        let value = json!({"conclusion":"Review finished", "findings":[{
            "severity":"critical", "text":"Important regression", "evidence":[]
        }]});
        let omitted = (0..1000)
            .map(|n| format!("src/long_module_{n}.rs: omitted at batch limit"))
            .collect::<Vec<_>>();
        for budget in [128, 256, 1000] {
            let result = compact_review_result(&value, budget, 32, false, &omitted);
            assert!(workspace::estimate_tokens(&result) <= budget as usize);
            assert!(result.contains("omitted=1000"));
            assert!(result.contains("Important regression"));
        }
        assert!(compact_primary_result(&value.to_string(), 0, None).is_empty());
    }

    #[test]
    fn compact_debug_result_keeps_diagnosis_and_verified_read_next() {
        let selection = DeepSelection {
            files: Vec::new(),
            symbols: vec![workspace::SymbolCandidate {
                path: "src/jobs.rs".to_owned(),
                label: "fn spawn_worker(job_id: &str)".to_owned(),
                start_line: 400,
                end_line: 445,
            }],
            truncated: false,
            used_llm: false,
        };
        let raw = json!({
            "diagnosis": "Worker startup fails because the executable path no longer exists.",
            "confidence": "high",
            "root_cause": "A transient executable path was replaced.",
            "execution_path": ["tools/call -> create job -> spawn_worker -> exec failure"],
            "verification": ["Inspect the stable worker executable path before spawning."],
            "read_next": [{
                "path": "src/jobs.rs",
                "symbol": "fn spawn_worker(job_id: &str)",
                "lines": "400-445",
                "reason": "Worker process creation boundary."
            }]
        })
        .to_string();
        let result = compact_primary_result(&raw, 300, Some(&selection));
        assert!(result.contains("DIAGNOSIS"));
        assert!(result.contains("ROOT CAUSE"));
        assert!(result.contains("HOW TO VERIFY"));
        assert!(result.contains("src/jobs.rs:400-445"));
    }

    #[test]
    fn compact_primary_result_keeps_valid_read_range_and_respects_budget() {
        let selection = DeepSelection {
            files: Vec::new(),
            symbols: vec![workspace::SymbolCandidate {
                path: "src/jobs.rs".to_owned(),
                label: "fn spawn_worker(job_id: &str)".to_owned(),
                start_line: 400,
                end_line: 445,
            }],
            truncated: false,
            used_llm: false,
        };
        let raw = json!({
            "conclusion": "The worker launch path is the relevant failure boundary.",
            "findings": [{
                "severity": "high",
                "text": "The configured executable path can disappear after replacement.",
                "evidence": ["src/jobs.rs:400-445"]
            }],
            "actions": ["Use the stable executable path."],
            "read_next": [{
                "path": "src/jobs.rs",
                "symbol": "fn spawn_worker(job_id: &str)",
                "lines": "400-445",
                "reason": "Contains worker process creation."
            }]
        })
        .to_string();
        let result = compact_primary_result(&raw, 128, Some(&selection));
        assert!(result.contains("READ_NEXT"));
        assert!(result.contains("src/jobs.rs:400-445"));
        assert!(workspace::estimate_tokens(&result) <= 128);
    }

    #[test]
    fn compact_primary_result_rejects_invented_read_range() {
        let selection = DeepSelection {
            files: Vec::new(),
            symbols: vec![workspace::SymbolCandidate {
                path: "src/jobs.rs".to_owned(),
                label: "fn spawn_worker(job_id: &str)".to_owned(),
                start_line: 400,
                end_line: 445,
            }],
            truncated: false,
            used_llm: false,
        };
        let raw = json!({
            "conclusion": "Check the worker launcher.",
            "read_next": [{
                "path": "src/jobs.rs",
                "lines": "1-9999",
                "reason": "invented"
            }]
        })
        .to_string();
        let result = compact_primary_result(&raw, 120, Some(&selection));
        assert!(result.contains("src/jobs.rs:400-445"));
        assert!(!result.contains("1-9999"));
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
