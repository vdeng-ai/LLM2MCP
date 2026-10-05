# Diagnostics and host verification

`llm2mcp doctor` verifies actual inference, legacy stdio initialize/tools/list/ping and registration files. These checks do not establish that Cursor, Codex or another host successfully executes tools.

## Opt-in deep check

```bash
llm2mcp doctor --json --deep --workspace /absolute/project --path src/main.rs
```

This sends the explicitly selected readable source to the configured analysis model and may incur API cost. It forces asynchronous analysis **only in the diagnostic child process**, verifies modern `server/discover` Tasks negotiation, creates a real task through `tools/call`, disconnects/reconnects, polls to completion and compares the persisted result with portable `job_result`. A separate synthetic queued handle verifies persisted cancellation intent and a local simulated worker acknowledgment without an additional inference call; active-worker cancellation is covered separately by the end-to-end suite. Both probe jobs appear in history. Failures cancel the probe's pending work; the CLI exits nonzero on a failed required deep check.

`--profile NAME` selects the default inference/profile route; a tool's explicit `model_profile` remains authoritative. `LLM2MCP_ACTIVE_PROFILE` and `LLM2MCP_ANALYZE_EXECUTION=sync|async|auto` are also available as process-scoped overrides. Diagnostics do not rewrite saved configuration.

End-to-end tests additionally exercise cancellation of an active HTTP request through modern Tasks, missing-capability rejection, task completion after reconnect and portable-result equivalence. No `input_required` flow is advertised: current jobs do not request user input.

## Verify in a real host

After installing the MCP entry in the desired host, start a new host session in a small test repository. Ask it to call `analyze` on one explicit source path, retain the returned task/job ID, retrieve the completed result and verify the cited source. For a long request, confirm that ping/other host work remains responsive and cancellation stops the local task. Reconnect and retrieve an existing completed result. A host without modern Tasks opt-in should use `job_status`, `job_result` and `job_cancel`.

Record host name/version, executable path, workspace, negotiated protocol and the actual task/job result. A registration badge, local doctor success, or another host's success cannot substitute for this check. See the [official Tasks extension](https://modelcontextprotocol.io/extensions/tasks/overview) for negotiation and lifecycle behavior.
