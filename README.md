# LLM2MCP

[English](README.md) | [简体中文](README.zh-CN.md)

LLM2MCP is a cross-platform, single-binary local MCP bridge that turns any OpenAI-compatible LLM API into a secondary coding model for AI coding agents.

It is designed for context offloading: instead of making the primary coding agent read large directories, large files, or large diffs itself, LLM2MCP reads the selected code locally, sends that context to your configured LLM, and returns only a compact analysis, implementation plan, or review result.

LLM2MCP uses local stdio MCP. It does not require a local HTTP server and does not override the model configuration of Cursor, Codex, Claude Code, Grok Build, or other coding agents.

## Features

- Single binary for Windows, Linux, and macOS.
- English and Simplified Chinese GUI.
- Configure OpenAI-compatible API endpoint, API key, model, and reasoning transport.
- Configure reasoning strength, execution mode (`Sync / Auto / Async`), and output budget independently for each MCP tool.
- Integrations for multiple AI coding agents plus a generic stdio MCP mode.
- Read-only workspace access with path sandboxing.
- Respects `.gitignore` and excludes common credential/secret files by default.
- Offloads large-context analysis to a self-hosted or custom LLM so the primary coding agent only consumes compact results.
- Generates repository documentation with bounded Map→Reduce passes and incrementally updates docs from Git changes.
- Durable background jobs let slow local models run for minutes without holding a single MCP `tools/call` open.
- Supports the MCP `2026-07-28` Tasks extension when the host opts in, with portable `job_status / job_result / job_cancel` fallback tools for older hosts.

## Supported AI Coding Agents

### Cursor

LLM2MCP merges a user-level MCP entry into `~/.cursor/mcp.json`. Cursor passes the current workspace through `${workspaceFolder}`.

### Claude Code

LLM2MCP installs a user-level stdio MCP server through the `claude mcp` command.

### Codex

LLM2MCP installs a stdio MCP server through the `codex mcp` command.

### Grok Build

LLM2MCP installs a stdio MCP server through the `grok mcp` command.

### Pi

Pi currently requires an MCP extension. LLM2MCP can attempt to install `pi-mcp-extension` and then configure a user-level MCP entry.

### Other MCP Clients

Any coding agent that can launch a local stdio MCP server can use:

```bash
llm2mcp mcp
```

Without `--workspace`, LLM2MCP uses the process working directory as the workspace root.

You can also set it explicitly:

```bash
llm2mcp mcp --workspace /path/to/project
```

## MCP Tools

LLM2MCP exposes five primary read-only tools plus three portable background-job tools:

| Tool | Best for | Required parameters | Optional parameters | Default reasoning | Default execution |
| --- | --- | --- | --- | --- | --- |
| `analyze` | Root-cause analysis, architecture analysis, large file/directory inspection | `task` | `paths` | Medium | Auto |
| `plan` | Independent implementation planning and second opinions | `task` | `paths` | XHigh | Auto |
| `review_diff` | Reviewing tracked Git changes without loading the full diff into the primary agent | None | `task`, `base_ref` | Medium | Sync |
| `document_repo` | Scanning a repository and generating grounded project documentation with bounded Map→Reduce passes | None | `document_type`, `paths`, `audience`, `language` | Medium | Async |
| `update_docs` | Updating existing README/docs from Git changes without rescanning the entire repository | None | `base_ref`, `target_ref`, `docs`, `language` | XHigh | Async |

Each primary tool has its own configurable reasoning level (`Off / Low / Medium / XHigh`), execution mode (`Sync / Auto / Async`), and maximum output-token budget in the GUI.

Portable job tools are always fast and read-only:

- `job_status(job_id)` — retrieve stage/progress/status.
- `job_result(job_id)` — retrieve the original completed tool result; while still running, returns current status.
- `job_cancel(job_id)` — request cooperative cancellation.

### `analyze`

Use `analyze` when the primary coding agent would otherwise need to read a large amount of source code, logs, configuration, or multiple related modules.

Parameters:

- `task` — required. Describe what to investigate and what questions to answer.
- `paths` — optional array of workspace-relative files or directories. If omitted or empty, LLM2MCP only sends project overview files such as README and common manifests instead of recursively sending the whole repository.

Example agent instruction:

```text
Do not read the whole src directory first.
Call LLM2MCP analyze on src/auth and src/session.
Find the root cause of intermittent login failures, cite the relevant files/symbols,
and return only the files I should inspect next.
```

Typical tool arguments:

```json
{
  "task": "Find the root cause of intermittent login failures and identify the files that need changes",
  "paths": ["src/auth", "src/session"]
}
```

The result is structured as a compact engineering report containing conclusions, evidence, risks/root causes, recommended actions, and the next files the primary agent should inspect.

### `plan`

Use `plan` before a large implementation when you want an independent plan from your local/custom LLM without making the primary coding agent load the entire relevant codebase first.

Parameters:

- `task` — required. Describe the feature, bug fix, migration, or refactor to plan.
- `paths` — optional array of workspace-relative files or directories that should be considered.

Example agent instruction:

```text
Before implementing this feature, call LLM2MCP plan.
Use src/cache, src/image, and Cargo.toml as context.
Ask the secondary LLM for the smallest implementation plan, risks, affected modules, and tests.
Then use that plan as a second opinion rather than blindly following it.
```

Typical tool arguments:

```json
{
  "task": "Design a persistent thumbnail cache with minimal changes and no regression to existing image loading",
  "paths": ["src/cache", "src/image", "Cargo.toml"]
}
```

The result focuses on goals and constraints, proposed implementation, files/modules to change, ordered steps, risks, compatibility concerns, and verification.

### `review_diff`

Use `review_diff` after making changes. LLM2MCP reads the Git diff locally and sends it directly to the configured LLM, so the primary coding agent does not need to ingest the entire diff first.

Parameters:

- `task` — optional. Defaults to reviewing the current changes for correctness and regressions.
- `base_ref` — optional. Git base revision used by `git diff`. Defaults to `HEAD`.

Example agent instruction:

```text
Call LLM2MCP review_diff before finishing.
Review the current tracked changes for correctness bugs, regressions, concurrency issues,
API compatibility, and missing tests. Only bring the important findings back into this chat.
```

Typical tool arguments:

```json
{
  "task": "Review the current changes for correctness, race conditions, regressions, and missing tests",
  "base_ref": "HEAD"
}
```

If there is no tracked diff, the tool returns that no tracked Git diff was found.

### `document_repo`

Use `document_repo` when you want the secondary LLM to understand a repository and produce developer-facing project documentation. Unlike `analyze`, an empty `paths` list intentionally scans the repository recursively. LLM2MCP splits readable source files into bounded chunks, summarizes each chunk, and then synthesizes the final documentation from those repository-map summaries.

Parameters:

- `document_type` — optional. One of `overview`, `architecture`, `modules`, `api`, `developer`, `deployment`, or `full`. Defaults to `overview`.
- `paths` — optional workspace-relative files/directories. Empty means scan the repository.
- `audience` — optional free-form audience such as `developer`, `new_contributor`, or `operator`. Defaults to `developer`.
- `language` — optional. `auto`, `english`, or `simplified_chinese`. Defaults to `auto`.

Example agent instruction:

```text
Call LLM2MCP document_repo instead of reading the whole repository yourself.
Generate architecture documentation for developers, include supported Mermaid diagrams,
and cite concrete files/modules/symbols. Return the generated Markdown to me; do not write files automatically.
```

Typical tool arguments:

```json
{
  "document_type": "architecture",
  "audience": "developer",
  "language": "english"
}
```

`full` mode asks for a documentation bundle with these suggested targets:

```text
docs/PROJECT_OVERVIEW.md
docs/ARCHITECTURE.md
docs/MODULES.md
docs/API.md
docs/DEVELOPMENT.md
docs/DEPLOYMENT.md
```

The MCP remains read-only. Generated documents are returned with `===== DOCUMENT: path =====` delimiters so the primary coding agent can review them and decide whether to write them.

`document_repo`, especially `full` mode on a large repository, is intentionally long-running and may require multiple LLM calls. It defaults to **Async**, so the original MCP call returns a durable job/task handle quickly instead of waiting minutes. Map passes use **Low** reasoning and a compact ~800–1200-token budget for evidence extraction. Repository Map calls run with configurable concurrency (default **2**) and each successful chunk summary is immediately stored in a content-addressed checkpoint cache. If a later Reduce step fails, retrying the same repository only recomputes changed/cache-miss chunks instead of rescanning everything.

The final Reduce pass defaults to **Medium** reasoning. If the configured reasoning level returns no final `message.content` (for example `finish_reason=length` with only `reasoning_content`), LLM2MCP automatically retries the same cached Map summaries with lower reasoning (`XHigh → Medium → Low`, or `Medium → Low`). If every Reduce attempt still fails, the Job is marked `failed` at `reduce_failed` instead of incorrectly reporting `completed`. Error details include `finish_reason`, prompt/completion token counts when available, and reasoning/content sizes.

### `update_docs`

Use `update_docs` after code changes when existing documentation may be stale. LLM2MCP reads the Git diff locally, discovers README/docs Markdown by default, and asks the configured LLM to return complete replacement Markdown only for affected documents. This tool requires the workspace to be a Git repository with the referenced commits/tags available.

Parameters:

- `base_ref` — optional. Defaults to `HEAD`.
- `target_ref` — optional. Defaults to `WORKTREE`, meaning current staged/unstaged tracked changes. It can also be a Git ref such as `main`, `v0.2.0`, or another commit/tag.
- `docs` — optional workspace-relative Markdown files. Empty auto-discovers `README.md`, `README.zh-CN.md`, and `docs/**/*.md`.
- `language` — optional. `auto`, `english`, or `simplified_chinese`.

Example agent instruction:

```text
Call LLM2MCP update_docs for HEAD -> WORKTREE.
Check whether the current code changes make README or docs stale.
Only return documents that really need changes, then let me review them before writing anything.
```

Typical tool arguments:

```json
{
  "base_ref": "HEAD",
  "target_ref": "WORKTREE",
  "language": "auto"
}
```

`update_docs` also defaults to **Async**, so slow documentation synthesis does not keep the host's original MCP call open.

For release-to-release documentation maintenance you can compare refs directly:

```json
{
  "base_ref": "v0.1.0",
  "target_ref": "v0.2.0",
  "docs": ["README.md", "docs/ARCHITECTURE.md"]
}
```

## Long-running jobs and slow local models

Local/self-hosted models may need minutes for repository-scale work. LLM2MCP therefore does not rely on a single long-lived `tools/call` for slow tools.

Execution modes are configurable per primary tool:

- **Sync** — keep the original MCP call open until the LLM returns. Best for small, predictable requests.
- **Auto** — run small requests synchronously and switch larger directory/multi-file requests to a background job.
- **Async** — durably create a background job and return immediately. This is the default for `document_repo` and `update_docs`.

For legacy or Tasks-unaware MCP hosts, the flow is:

```text
tools/call document_repo
    ↓ returns quickly
job_id
    ↓
job_status(job_id)
    ↓ repeat when needed
job_result(job_id)
```

A job is written to LLM2MCP's local application-data directory before its handle is returned. Work is then executed by an independent `llm2mcp job-worker` subprocess, so the original MCP stdio process can exit or be restarted without losing the job result.

On Linux, LLM2MCP also handles an in-place rebuild/update while an MCP process is still running. If `current_exe()` resolves to a deleted executable inode (for example `target/debug/llm2mcp (deleted)` after `cargo build` replaced the development binary), the running MCP process launches its same-version worker through `/proc/self/exe` instead of failing with `No such file or directory`. After updating LLM2MCP itself, restart/reload the MCP connection once so the host uses the new server version.

For MCP hosts that opt into the `io.modelcontextprotocol/tasks` extension on protocol `2026-07-28`, LLM2MCP instead returns a standard `resultType: "task"` handle. The host can poll with `tasks/get`; the completed task contains the original tool result. The portable `job_*` tools remain available for hosts that do not support Tasks.

Job progress is persistent and stage-based, for example:

```text
scanning repository
mapping repository 3/8
synthesizing documentation
completed
```

`job_cancel` / `tasks/cancel` are cooperative. Cancellation is checked between stages and LLM calls; an HTTP request already executing against the configured LLM may finish before cancellation takes effect.

Jobs are retained for seven days by default and old job state/log files are cleaned up opportunistically when LLM2MCP starts.

As a secondary safety measure, the integration installer attempts to configure longer MCP tool timeouts where the target agent exposes such a setting. Async jobs remain the primary protection against host-side hard timeouts, especially for hosts where the timeout cannot be reliably changed.

## Recommended Context-Offloading Workflow

The biggest benefit comes from avoiding duplicate context consumption.

Recommended:

```text
Primary coding agent
    ↓ passes task + paths only
LLM2MCP
    ↓ reads large local source context
Configured LLM API
    ↓ returns a compact 2K–4K style report
Primary coding agent
    ↓ opens only the few files actually needed
Implementation
    ↓
LLM2MCP review_diff
```

Avoid this pattern:

```text
Primary coding agent reads 50K tokens of source
    ↓
Then calls LLM2MCP on the same 50K tokens
```

That duplicates work and reduces the token-saving benefit.

A good instruction to the primary agent is:

```text
Prefer LLM2MCP for large-context discovery. Do not read entire directories before calling it.
Pass workspace-relative paths to the tool, then only open the files identified as relevant in its report.
```

## LLM Compatibility

LLM2MCP currently uses OpenAI-compatible `POST /v1/chat/completions` and works with services such as LiteLLM, llama.cpp, vLLM, Ollama-compatible gateways, and other compatible APIs.

Supported reasoning transports:

- `None`
- `OpenAI reasoning_effort`
- `Qwen chat_template_kwargs`
- `thinking parameter`

Reasoning can be configured independently per tool. For example:

- `analyze` → Medium
- `plan` → XHigh
- `review_diff` → Medium
- `document_repo` → Medium
- `update_docs` → XHigh

## Development

Run the GUI:

```bash
cargo run
```

Then:

1. Configure the LLM API endpoint, model, and optional API key.
2. Click **Test Connection**.
3. Choose the target AI coding agent in the integrations section.
4. Click **Install / Update**.
5. Restart or refresh the coding agent's MCP list.

You can also test stdio MCP directly:

```bash
llm2mcp mcp --workspace /path/to/project
```

> In stdio mode, stdout is reserved exclusively for MCP JSON-RPC. Runtime logs must go to stderr.

## Security Model

LLM2MCP 0.1 is read-only by design. It does not provide arbitrary shell execution, file writes, Git commits, or pushes.

Requested paths are canonicalized and must remain inside the current workspace. Symlink escapes are rejected.

Common sensitive files are excluded by default, including:

- `.env*`
- `*.pem`
- `*.key`
- `id_rsa` / `id_ed25519`
- `credentials*`
- `secrets*`

Selected source code is still sent to the LLM API you configure. Make sure that endpoint has an appropriate data-handling and access-control policy for your code.

## Project Status

LLM2MCP is currently in the `0.1.0` MVP stage. See [ROADMAP.md](docs/ROADMAP.md) and [ARCHITECTURE.md](docs/ARCHITECTURE.md).

## License

MIT
