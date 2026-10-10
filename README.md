# LLM2MCP

[English](README.md) | [简体中文](README.zh-CN.md)

**Give Cursor a second LLM through MCP—without replacing Cursor's primary model.**

LLM2MCP is a cross-platform, single-binary local MCP bridge that makes a self-hosted or third-party OpenAI-compatible LLM available to Cursor as a **secondary coding model**. Cursor remains the main agent and executes edits; LLM2MCP reads selected workspace evidence locally and returns focused analysis, debugging, plans, reviews, or generated documentation.

**Why this exists:** Some AI coding tools restrict which models or custom API endpoints can power their built-in agent. Cursor does support bring-your-own API keys and some custom OpenAI-compatible endpoint setups, but those capabilities are not universal across models and features (for example, Cursor Tab continues to use its own models). LLM2MCP takes a different route: the primary agent invokes your separate LLM **as an MCP tool**, without requiring it to become the primary chat or autocomplete model.

This also enables **context offloading**: instead of having Cursor ingest entire directories, huge files, or large diffs, LLM2MCP sends bounded source evidence directly to your configured model and returns only the parts the primary agent needs. It uses local stdio MCP—no local HTTP listener, model proxy, or Cursor model override is required.

## Start with Cursor

1. [Install LLM2MCP](https://github.com/vdeng-ai/LLM2MCP/releases) for your operating system.
2. In the **LLM API** tab, enter your OpenAI-compatible endpoint, model, and API key (if required); run **Test connection**.
3. In **AI coding agents**, select **Cursor** and click **Install / Update**. Reload Cursor and check that the `llm2mcp` MCP server/tools are enabled.
4. In Cursor Agent, try: *"Before modifying code, call LLM2MCP analyze on src/ and identify the relevant modules. Open only the files it recommends, then implement the fix."*

**[Cursor setup, example prompts, boundaries, and troubleshooting →](docs/CURSOR.md)** · [简体中文指南](docs/CURSOR.zh-CN.md)

**Other hosts:** Existing Codex, Claude Code, Grok Build, and Pi integrations remain available, as does generic stdio MCP. Their value is the **independent second opinion and context offload**, even if they already support direct custom LLM APIs. MCP-capable hosts with restricted model choices may also benefit, but a host-specific installation must be verified.

## Features

- Single binary for Windows, Linux, and macOS.
- English and Simplified Chinese GUI with six module tabs (`LLM API / Context limits / Per-tool reasoning / System Prompt / Job history / AI coding agents`); native window size/position, UI/text zoom, and the last active tab are restored across launches.
- Configure OpenAI-compatible API endpoint, API key, model, reasoning transport, and an optional global system-prompt prefix; supports refreshing the OpenAI-compatible `/models` list.
- Configure reasoning strength, execution mode (`Sync / Auto / Async`), and output budget independently for each MCP tool.
- Integrations for multiple AI coding agents plus a generic stdio MCP mode.
- Read-only workspace access with path sandboxing.
- Respects `.gitignore` and excludes common credential/secret files by default.
- Offloads large-context analysis to a self-hosted or custom LLM so the primary coding agent only consumes compact results.
- Generates repository documentation with bounded Map→Reduce passes and incrementally updates docs from Git changes.
- Durable background jobs let slow local models run for minutes without holding a single MCP `tools/call` open.
- Supports the MCP `2026-07-28` Tasks extension when the host opts in, with portable `job_status / job_result / job_cancel` fallback tools for older hosts.

## Runtime and diagnostics

- Responsive background operations with cancellation; concurrent stdio requests and bounded job/HTTP scheduling.
- Named model profiles with per-tool/Discovery/Map routing, AST-assisted discovery and Chinese intent matching.
- Real inference and local stdio diagnostics (`llm2mcp doctor`), cache cleanup, and actionable job history with usage/cost estimates.

See [runtime configuration and usage](docs/RUNTIME.md) for examples, migration, measurements and limits.

## Context integrity in 0.3

Complete-file scanning and batched diff review expose coverage instead of silently dropping late evidence. Output validation rejects incomplete results; source budgets follow each routed model. `update_docs` produces hashed exact-fragment edits that preserve unseen document sections.

See [integrity and migration](docs/INTEGRITY.md), [paired effectiveness evaluation](docs/EVALUATION.md), and [deep MCP/host diagnostics](docs/DIAGNOSTICS.md).

## Installation

Download the package for your platform from GitHub Releases. Ubuntu/Debian x86_64 users can use the native `.deb` package:

```bash
sudo apt install ./LLM2MCP_<version>_amd64.deb
```

The `.deb` installs the application and desktop entry through the system package manager. Linux also provides an AppImage for portable use. Windows provides an NSIS installer, while macOS provides Apple Silicon and Intel DMGs.

AppImage builds can install signed updates in place from the GUI. Debian-package installs can still check for updates, but upgrading is intentionally done by installing the newer `.deb` so `/usr/bin` remains managed by APT/dpkg.

## Supported AI Coding Agents

**Install / Update** first refreshes a stable per-user LLM2MCP executable, then registers that stable path with the selected coding agent. This prevents MCP entries from pointing at transient development/build locations that can disappear after a rebuild or upgrade. When LLM2MCP directly edits JSON/TOML client configuration, it keeps the previous file as `*.llm2mcp.bak` and replaces the config through an atomic temp-file write.

### Cursor (recommended starting point)

LLM2MCP merges a user-level MCP entry into `~/.cursor/mcp.json`. Cursor passes the current workspace through `${workspaceFolder}`. Follow the [dedicated Cursor guide](docs/CURSOR.md) for registration, tool use, and limitations. This does **not** change Cursor's primary model or Tab completion.

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

The GUI can copy a generic `mcpServers.llm2mcp` JSON template that points to the stable per-user executable, which is useful for MCP hosts without a dedicated installer.

## MCP Tools

LLM2MCP exposes six primary read-only tools plus three portable background-job tools:

| Tool | Best for | Required parameters | Optional parameters | Default reasoning | Default execution |
| --- | --- | --- | --- | --- | --- |
| `analyze` | Root-cause analysis, architecture analysis, large file/directory inspection | `task` | `paths`, `include`, `exclude` | Medium | Auto |
| `debug_issue` | Diagnosing a concrete malfunction from symptoms, logs, source evidence, and optional recent Git changes | `issue` | `paths`, `logs`, `expected`, `actual`, `recent_changes`, `include`, `exclude` | XHigh | Auto |
| `plan` | Independent implementation planning and second opinions | `task` | `paths`, `include`, `exclude` | XHigh | Auto |
| `review_diff` | Reviewing tracked Git changes without loading the full diff into the primary agent | None | `task`, `base_ref` | Medium | Sync |
| `document_repo` | Scanning a repository and generating grounded project documentation with bounded Map→Reduce passes | None | `document_type`, `paths`, `include`, `exclude`, `audience`, `language` | Medium | Async |
| `update_docs` | Updating existing README/docs from Git changes without rescanning the entire repository | None | `base_ref`, `target_ref`, `docs`, `language` | XHigh | Async |

Each primary tool has its own configurable reasoning level (`Off / Low / Medium / XHigh`), execution mode (`Sync / Auto / Async`), and secondary-LLM output-token budget in the GUI. `analyze`, `debug_issue`, `plan`, and `review_diff` also have a separate **Primary return** budget: the secondary model can use a larger internal answer budget, while LLM2MCP locally compacts the structured result before it enters the primary coding agent's context. Defaults are 800 tokens for `analyze`, 1400 for `debug_issue`, 1200 for `plan`, and 1000 for `review_diff`; documentation tools return their full generated artifact. Input source limits are token-estimate based as well: the GUI separately controls the overall source budget, per-file budget, and lightweight discovery-index budget. The estimator is intentionally provider-neutral rather than pretending to be the exact tokenizer for every OpenAI-compatible model.

Portable job tools are always fast and read-only:

- `job_status(job_id)` — retrieve stage/progress/status.
- `job_result(job_id)` — retrieve the original completed tool result; while still running, returns current status.
- `job_cancel(job_id)` — request cooperative cancellation.

### `analyze`

Use `analyze` when the primary coding agent would otherwise need to read a large amount of source code, logs, configuration, or multiple related modules.

Parameters:

- `task` — required. Describe what to investigate and what questions to answer.
- `paths` — optional array of workspace-relative files or directories. A directory, multiple paths, or an empty list triggers a lightweight repository discovery pass before deep source reads. A single explicit file skips discovery and is read directly.
- `include` — optional glob list such as `src/**/*.rs`; only matching source candidates are considered.
- `exclude` — optional glob list such as `tests/**` or `*.generated.ts`; exclusions win over includes.

For discovery, LLM2MCP locally builds a bounded file/symbol index with exact symbol line ranges. It first tries local lexical/symbol scoring; strong matches skip the extra discovery LLM call entirely. Ambiguous tasks use a short Low-reasoning rerank over the lightweight index. Deep context then prefers the selected symbol ranges, with numbered source lines and only a small file-prelude/import window, instead of reading whole files. Paths or line ranges invented by the model are rejected because selections must match the local candidate index.

The file/symbol index is persistent across jobs. LLM2MCP stores a per-workspace Repository Symbol Index in its local application-data directory and uses file size + nanosecond mtime as a fast unchanged-file stamp. Unchanged files reuse their existing SHA-256 and symbol ranges without rereading source contents; only changed/new files are re-read, content-hashed, and reparsed. Selected source evidence is cached separately by **file SHA-256 + exact line segment**, so a later `analyze` or `plan` can reuse the same symbol/prelude evidence without reopening and slicing unchanged files. Any content change produces a different evidence key, and stale selected ranges are revalidated against the current symbol index before use. Cache files stay local to LLM2MCP and are not returned to the primary coding agent as history; Job status/history exposes symbol-index and evidence-cache hit/miss counters for verification.

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

The secondary model returns a structured analysis, and LLM2MCP compacts it locally to the configured Primary return budget without a second LLM call. The return budget reserves space for the summary, severity-ranked findings and validated `READ_NEXT` entries, for example `src/jobs.rs:400-445 — fn spawn_worker(...)`, so the primary coding agent can open only the relevant range rather than ingesting the entire file.

### `debug_issue`

Use `debug_issue` when the program is already malfunctioning and the goal is to explain **why it fails**. It reuses the same bounded file/symbol discovery and Evidence Cache as `analyze`, but its output is specialized for diagnosis: confidence, likely root cause, execution path, intermittent-behavior explanation, alternative hypotheses, verification steps, likely fix area, and validated `READ_NEXT` evidence.

Parameters:

- `issue` — required symptom or malfunction description.
- `paths` — optional files/directories to prioritize.
- `logs` — optional stack trace, runtime/compiler error, HTTP error, browser console output, service logs, or other observed evidence.
- `expected` / `actual` — optional expected and actual behavior.
- `recent_changes` — optional boolean; when true, the current tracked Git diff against `HEAD` is included as debugging evidence.
- `include` / `exclude` — optional source globs.

Large logs, recent Git changes, or broad paths automatically favor Async under `Auto`. Runtime evidence shares the configured source-token budget with source context, so logs/diffs cannot silently create an unbounded request. `debug_issue` remains read-only: it does not execute programs, run tests, modify files, or restart services; it returns verification suggestions to the primary coding agent instead.

Typical tool arguments:

```json
{
  "issue": "The API worker intermittently fails to start with No such file or directory",
  "paths": ["src/jobs.rs", "src/install.rs"],
  "logs": "failed to start job worker: No such file or directory (os error 2)",
  "expected": "The durable background worker starts after the MCP request returns",
  "actual": "The job is immediately marked failed",
  "recent_changes": true
}
```

### `plan`

Use `plan` before a large implementation when you want an independent plan from your local/custom LLM without making the primary coding agent load the entire relevant codebase first.

Parameters:

- `task` — required. Describe the feature, bug fix, migration, or refactor to plan.
- `paths` — optional array of workspace-relative files or directories that should be considered. Directory/multi-path/empty requests use the same local-scoring → optional LLM rerank → symbol-range context selection path as `analyze` before deep reads.
- `include` — optional include globs.
- `exclude` — optional exclude globs; exclusions win over includes.

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
- `include` — optional include globs applied during the local repository scan.
- `exclude` — optional exclude globs applied after includes.
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

Use `update_docs` after code changes when existing documentation may be stale. LLM2MCP reads the Git diff locally, discovers README/docs Markdown by default, and returns validated exact-fragment edits with original SHA-256 hashes. Unseen original sections are preserved. Check each hash before applying edits; see [safe update format](docs/INTEGRITY.md). This tool requires the workspace to be a Git repository with the referenced commits/tags available.

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

Coding-agent integrations point to the stable per-user executable rather than a transient build path, which removes the normal `target/debug/...` rebuild failure mode for background workers. Linux also keeps a `/proc/self/exe` fallback for development or unusual in-place updates where an already-running MCP process sees its original executable path as deleted. After updating LLM2MCP itself, restart/reload the MCP connection once so the host uses the new server version.

For MCP hosts that opt into the `io.modelcontextprotocol/tasks` extension on protocol `2026-07-28`, LLM2MCP instead returns a standard `resultType: "task"` handle. The host can poll with `tasks/get`; the completed task contains the original tool result. The portable `job_*` tools remain available for hosts that do not support Tasks.

Job progress is persistent and stage-based, for example:

```text
scanning repository
mapping repository 3/8
synthesizing documentation
completed
```

`job_cancel` / `tasks/cancel` are cooperative. Cancellation is checked between stages and LLM calls; an HTTP request already executing against the configured LLM may finish before cancellation takes effect.

Jobs are retained for seven days by default and old job state/log files are cleaned up opportunistically when LLM2MCP starts. The GUI can change the retention TTL and host polling interval used by newly created jobs. Durable Job records also accumulate runtime, LLM call count, prompt/completion token usage when the provider reports it, the last response diagnostics, and a targeted hint for common authentication, rate-limit, connection, timeout, output-budget, or source-filter failures. The GUI exposes the most recent Job records in a refreshable history panel without storing an additional copy of source-code bodies.

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

This source tree is at version `0.4.2`. See [ROADMAP.md](docs/ROADMAP.md) and [ARCHITECTURE.md](docs/ARCHITECTURE.md). Development and release documentation is kept in [DEVELOPMENT.md](docs/DEVELOPMENT.md) and [RELEASE.md](docs/RELEASE.md).

### Quality fixes in 0.3.1

The complete review response now shares one return budget, including coverage. Compaction prioritizes severe findings, runtime citations are checked against supplied logs/diffs, and documentation updates select relevant sections even late in a file. See [0.3.1 release notes](docs/RELEASE_NOTES-0.3.1.md).

### Bounded retrieval and continuations in 0.4.0

Analysis, debug and planning can request one supplemental retrieval round (`allow_supplement`). Use `result_page` to inspect a Job's complete validated result without another model call. Large repository documentation scans return a durable `scan_cursor`; `continue_scan` resumes the next file/segment and uses accumulated Map evidence. See [usage and JSON examples](docs/CONTINUATIONS.md) and [0.4.0 release notes](docs/RELEASE_NOTES-0.4.0.md).

### Runtime efficiency in 0.4.1

Map workers immediately take the next pending chunk when one finishes, keeping the existing concurrency limits and ordered evidence. Analysis and supplemental queries share a task-local candidate snapshot; evidence is still checked against current source. Automatic cache pruning runs at most once per five minutes across processes for an unchanged policy; the `cache` command remains immediate. HTTP requests share a process-local runtime and connection pool, with per-request authentication, timeouts and cancellation.

For a large documentation scan, set `"synthesis":"final"` on the initial `document_repo` request. Continue every returned `scan_cursor`; intermediate pages save Maps without generating documentation, and the last page synthesizes the accumulated evidence once. Default `"each_page"` preserves intermediate documentation. See [usage](docs/CONTINUATIONS.md) and [0.4.1 release notes](docs/RELEASE_NOTES-0.4.1.md). These changes reduce avoidable work; real-model latency, quality and token savings still require [paired evaluation](docs/EVALUATION.md).

### Compiled matching in 0.4.2

Source include/exclude rules are compiled once and reused during traversal. Discovery, supplemental retrieval and document previews share a keyword matcher that uses Aho-Corasick for larger queries and retains native substring search for small queries. Symbol scores are computed once before sorting. Existing filter behavior, Chinese matching, overlapping keywords and result order are preserved. See [0.4.2 release notes](docs/RELEASE_NOTES-0.4.2.md) and the reproducible [local matching benchmark](docs/MATCHING_BENCHMARK.md).

## License

MIT
