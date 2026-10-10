# LLM2MCP for Cursor: use a second coding LLM through MCP

> Keep Cursor's existing primary agent while using your own OpenAI-compatible model to analyze a large codebase, investigate bugs, propose plans, review changes, and draft documentation.

## What it does—and does not do

**Cursor is not entirely closed to custom APIs.** Its supported BYOK providers and some OpenAI-compatible endpoint configurations can be useful, but capabilities differ across models/features. In particular, Cursor Tab continues to use Cursor-managed models.

LLM2MCP takes a separate approach: **Cursor remains the primary agent; your self-hosted or third-party LLM is a read-only MCP tool**. The primary agent can delegate a bounded repository investigation and receive a concise, validated result. This can be useful when native arbitrary model configuration is unavailable, limited, or simply not what you want.

```text
Cursor Agent (primary model; edits files)
       ↓ MCP tool call: task + workspace-relative paths
LLM2MCP (local, read-only workspace inspection)
       ↓ OpenAI-compatible /v1/chat/completions
Your secondary LLM
       ↓ analysis / diagnosis / plan / review
LLM2MCP (validation and bounded return)
       ↓
Cursor Agent (opens relevant files and implements changes)
```

LLM2MCP **does not** override Cursor Chat, Agent, or Tab models, proxy Cursor's model requests, bypass subscription restrictions, or make every Cursor feature use your endpoint.

## Quick start

1. Install LLM2MCP from [GitHub Releases](https://github.com/vdeng-ai/LLM2MCP/releases).
2. Open **LLM API**, set your OpenAI-compatible base URL (for example `http://127.0.0.1:8000/v1`), exact model ID, and an API key if required. Refresh the model list and run **Test connection**. Your endpoint must support **`POST /v1/chat/completions`**; a Responses-only endpoint is not sufficient.
3. In **AI coding agents**, select **Cursor** and click **Install / Update**. LLM2MCP installs a stable per-user executable and merges the MCP entry into `~/.cursor/mcp.json`, retaining other entries and backing up changed configuration with `*.llm2mcp.bak`.
4. Reload Cursor. In **Customize → MCPs** (menu names may vary by release), enable the `llm2mcp` tools. Use **Agent** mode and approve the tool call when prompted.
5. Try this in your project:

   > Before changing code, call LLM2MCP `analyze` on `src/`. Don't read the whole directory yourself first. Use its file/symbol findings to inspect only the necessary parts, then implement the change.

The `mcp` server is read-only. Cursor's own agent is responsible for file changes.

## Manual MCP entry (when not using the GUI)

Cursor supports global `~/.cursor/mcp.json` and project-level `.cursor/mcp.json` entries. Prefer the built-in installer, which uses the stable executable path instead of a temporary build location.

```json
{
  "mcpServers": {
    "llm2mcp": {
      "command": "/absolute/path/to/llm2mcp",
      "args": ["mcp", "--workspace", "${workspaceFolder}"]
    }
  }
}
```

Replace the executable path with a real absolute path (escape Windows backslashes in JSON). **Merge** this entry with existing `mcpServers`; do not overwrite other servers. Cursor expands `${workspaceFolder}` in MCP configuration.

## Recommended usage

| Goal | MCP tool | Notes |
| --- | --- | --- |
| Find relevant files/symbols in a large codebase | `analyze` | Send paths and a question instead of the full files |
| Diagnose a failure using logs and code evidence | `debug_issue` | Supply symptoms, expected/actual behavior, and relevant logs |
| Ask for an independent implementation plan | `plan` | Treat output as a second opinion, not an instruction to apply blindly |
| Review tracked Git changes | `review_diff` | New untracked files may not be included |
| Produce repository documentation | `document_repo` | Often async; generated docs are not written automatically |
| Update docs based on Git changes | `update_docs` | Review suggested edits before applying |

Optional Cursor User Rule:

```text
For large code exploration, cross-module refactors, or difficult debugging,
prefer LLM2MCP before reading an entire directory.
Pass a precise task and workspace-relative paths to the MCP tool.
Open only the key files/symbols it identifies and verify important claims.
Use review_diff after implementation when there are tracked Git changes.
If a tool returns a job_id, use job_status and job_result.
Do not invoke the secondary model for trivial questions.
```

## Troubleshooting and boundaries

- **Tools not visible:** reload Cursor, verify the executable path and MCP toggle, and confirm the server appears in the client. Registration alone does not prove Cursor invoked it.
- **401, unknown model, or timeouts:** test the LLM2MCP API key, endpoint, model, network access, and Chat Completions compatibility independently in the LLM2MCP GUI.
- **Slow models:** some tools return a durable job handle. Use `job_status(job_id)`, `job_result(job_id)`, or `job_cancel(job_id)`. Native MCP Tasks support depends on the host's negotiated capabilities.
- **Security:** LLM2MCP's workspace access is read-only, but selected source code is transmitted to **your configured LLM API**. Built-in secret filtering is best effort, not a guarantee.
- **Cost/performance:** the secondary inference request adds cost and latency; offloading is useful primarily when it avoids duplicating large contexts in the primary agent.

Run `llm2mcp doctor` for actual inference and stdio diagnostics. See [runtime and profiles](RUNTIME.md) for deeper configuration.

## Other MCP-capable coding tools

This guide is **Cursor-first**. Some tools with restricted native model selection still allow local MCP servers. Official documentation for legacy **Windsurf Cascade**, for instance, describes stdio MCP configuration; the newer Devin Local agent has different configuration. LLM2MCP does **not** currently offer a dedicated Windsurf one-click installer, so use its generic stdio MCP template and verify your specific host/version before claiming support.

Codex, Claude Code, and GitHub Copilot CLI have varying native BYOK/custom-provider options. In those clients, LLM2MCP's primary value is an independent **second opinion** and **context offload**, not the only means of using a custom LLM.

## References

- [Cursor BYOK and feature limitations](https://cursor.com/help/models-and-usage/api-keys)
- [Cursor MCP documentation](https://cursor.com/docs/mcp)
- [Legacy Cascade MCP documentation](https://docs.devin.ai/desktop/cascade/mcp)
- [LLM2MCP runtime and diagnostics](RUNTIME.md)
