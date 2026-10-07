# Runtime, profiles, diagnostics and job history

These features are available from the main branch; installing an older release does not enable them. Existing JSON configuration migrates with defaults and keeps the original endpoint/model when no named profile is selected.

## Quick start

In **LLM API**, set the endpoint and model, then run **Test connection**. This sends a small real inference request, probes this executable's stdio MCP (`initialize`, `tools/list`, `ping`), and reports client registration separately. A successful `/models` response alone does not pass inference. Registration is not proof that a particular host can invoke tools or supports Tasks.

Model listing, connection tests, client CLI commands and cache management run in background threads. The header shows a spinner and **Cancel**. Model listing has a 20-second timeout; inference diagnostics cap each HTTP request at 30 seconds. Client status commands have a 10-second timeout, install/remove commands 120 seconds, and the stdio probe 10 seconds. Cancel stops the pending local operation; an already-written client configuration is not rolled back.

The same diagnostics work without a desktop:

```bash
llm2mcp doctor
llm2mcp doctor --json
llm2mcp doctor --profile analysis --json
llm2mcp jobs
llm2mcp jobs --json > jobs.json
llm2mcp jobs --cancel JOB_ID
llm2mcp jobs --retry JOB_ID
llm2mcp cache
llm2mcp cache --clear
```

`doctor` exits nonzero if real inference or local stdio fails. Missing/unregistered optional clients are warnings. Probes use the configured API and can incur a small inference cost. `cache` inspects **and prunes** expired/oversized data; `--clear` removes source/index/map data and preserves job history.

## Named model profiles

Add a named profile in **LLM API**. Profiles have their own endpoint, API key, model, reasoning transport, timeout, context/output limits and compatibility parameters. **Per-tool reasoning** routes each tool plus Discovery and Document Map to a profile. The active profile is the default for tools; stage overrides fall back to the tool's effective profile when absent.

Example JSON fragment (merge with your existing config):

```json
{
  "profiles": [
    {
      "name": "analysis",
      "base_url": "http://127.0.0.1:6677/v1",
      "api_key": "",
      "model": "qwen3.8-27b",
      "reasoning_transport": "open_ai",
      "max_input_tokens": 262144,
      "max_output_tokens": 16384,
      "timeout_secs": 900,
      "send_temperature": true,
      "completion_token_parameter": "max_tokens",
      "input_usd_per_million": null,
      "output_usd_per_million": null
    },
    {
      "name": "fast",
      "base_url": "http://127.0.0.1:8000/v1",
      "model": "your-fast-model"
    }
  ],
  "active_profile": "analysis",
  "discovery_profile": "fast",
  "map_profile": "fast",
  "tools": {
    "analyze": {"reasoning": "medium", "max_output_tokens": 4000, "model_profile": "analysis"}
  },
  "max_concurrent_jobs": 2,
  "max_concurrent_requests": 4,
  "cache_max_mib": 256,
  "cache_ttl_days": 30
}
```

`max_input_tokens` is the **total context** ceiling, including reserved output and message overhead. The old top-level fallback uses `model_input_limit` / `model_output_limit`. Token counts are estimates, not a model-specific tokenizer. A request over the ceiling fails before sending. Set `send_temperature=false` for APIs that reject the parameter, and `completion_token_parameter="max_completion_tokens"` when required. Prices are optional USD per million tokens; null leaves cost unpriced. Profiles must have unique nonempty names, and routes must resolve before saving.

Environment variables `LLM2MCP_BASE_URL`, `LLM2MCP_MODEL`, `LLM2MCP_API_KEY` override the legacy fallback configuration; named profiles retain their own explicit values. `LLM2MCP_CONFIG_DIR` and `LLM2MCP_DATA_DIR` allow isolated/portable configuration and data directories, including integration tests. Configure them consistently for GUI and MCP processes if sharing queues/caches.

## Concurrency and cancellation

The stdio reader dispatches business calls to threads while continuing to serve ping, task status and cancellation. Up to eight business requests per stdio process can be in flight; excess/duplicate IDs receive a retryable busy error. JSON-RPC responses may arrive out of order and are matched by ID. Malformed JSON returns a parse error.

`notifications/cancelled` with `params.requestId` cancels an in-flight synchronous call. Durable async jobs have already returned a handle: use `job_cancel` or `tasks/cancel` with that handle. Dropping the HTTP future aborts local connection/body/retry waits. Whether a remote inference server stops GPU work depends on that server.

File-lock leases share job and HTTP concurrency caps across processes using the same data directory. Defaults are 2 running jobs and 4 HTTP requests, clamped to 1–16. Map workers obey the HTTP cap in addition to Map concurrency. The durable queue admits at most 64 pending/running jobs. Changing caps affects new slot acquisition; already-running work is allowed to finish.

Workers heartbeat every 3 seconds, including while queued. Missing heartbeats for over 30 seconds become `failed / worker_lost` (or cancelled if cancellation was pending) on startup, history refresh or status polling. A crashed worker releases slot locks automatically. Retry creates a new job from the original arguments using current configuration and reusable cache checkpoints; it does not resume execution in the middle of a model call. Old jobs without snapshot/heartbeat fields remain readable.

Async jobs persist a credential-free configuration snapshot so later model/budget edits do not change a queued job's routing. Workers restore keys from the current config only for matching profile names and endpoints. If a profile is removed or its endpoint changes, credentials are not copied to the old endpoint. Retry deliberately uses current routing.

## Retrieval and privacy

Relevant paths and symbol labels are ranked before the index token budget and 512-file candidate cap. Rust, Python, TypeScript/TSX and Go use Tree-sitter ranges; other text formats use a line-based fallback. A small one-hop static import/module expansion improves the rank of dependencies of the strongest matches. It is a lexical hint, not complete semantic dependency resolution. Cold discovery indexes all eligible source files; subsequent discovery reuses size/mtime-keyed indexes. Symbol requests can read later regions of a file. Files larger than 8 MiB and binary files are skipped. Per-file/source/Map chunk budgets still apply and truncation is reported.

Chinese intent aliases map requests such as 登录、认证、缓存、取消、超时 to English identifiers. Ambiguous tasks still use model reranking; aliases are not general Chinese segmentation or embeddings.

Sensitive paths (`.env*`, common credentials/key names and suffixes, `.aws`, `.ssh`, `.gnupg`, `.git`) are filtered from source and diff selection. Common credential assignments, Bearer tokens, key formats and private key blocks are masked in source/evidence caches, diff/log prompts, outbound messages and errors. Redaction preserves source line counts and quoted assignments. This is heuristic: it can mask nonsecret variables and cannot recognize every possible secret. Review sensitive repositories and use explicit include/exclude filters. Git diff disables external diff/textconv execution. Cache versions are bumped so older evidence is not reused; use `cache --clear` to remove previous cached files as well.

## Cache and measurement

Repository-map keys include endpoint, model, effective system-prefix and Map instructions, source paths/content, reasoning, temperature compatibility and output budget. Changing the prefix invalidates summaries. Defaults retain up to 256 MiB of source/index/map data for 30 days; cleanup removes expired/oldest data. This is an on-access bound, not a continuously enforced quota: concurrent writes can briefly exceed it. Lock/temp files are not counted or removed.

Job history refreshes every two seconds while open. Filter by workspace, tool or state; expand a row to view/copy results, export JSON to the clipboard, cancel active jobs or retry completed/failed/cancelled jobs. CLI JSON can be saved directly to a file.

Recorded measurements distinguish:

- `source_tokens` / `return_tokens`: estimated selected source and final response sizes; source selection may be partial and document returns can exceed source size.
- `selected_files` / `source_truncated`: selected file count and truncation signal; diff review records 0 files when only diff material is measured.
- `prompt_tokens` / `completion_tokens`: provider-reported secondary-model usage accumulated across logical model calls.
- `llm_wait_ms`, `used_models`, cache hits/misses and elapsed duration.
- `estimated_llm_cost_usd`: only calls with both prices and provider token usage; `priced_llm_calls / llm_calls` shows coverage. Internal HTTP retry attempts without usage cannot be priced.

Return/source compression is not observed savings in the primary model's billing. To compare offloading, run the same task against the same repository commit with and without the bridge and compare primary-provider usage, returned evidence quality and elapsed time. Preserve exported job JSON and record cold/warm cache conditions. Documentation tools intentionally return full artifacts.

## 验收清单（11 项）

| 项目 | 入口与验证 |
| --- | --- |
| 1. GUI 后台操作与取消 | 刷新模型、测试连接、客户端安装/移除；顶部取消 |
| 2. 敏感信息过滤 | 源码/符号证据/diff/log/出站提示词；凭据快照不落盘 |
| 3. MCP 并发与真正的 HTTP 取消 | 慢请求期间 ping 可用；取消通知中止本地等待 |
| 4. 全局调度与任务恢复 | 文件锁并发上限、队列上限、心跳、崩溃恢复和重试 |
| 5. 检索质量 | 预算前相关性排序、四语言语法树、静态导入一层扩展 |
| 6. 中文查询 | 中文意图到英文路径/标识符别名；保留模型重排 |
| 7. 缓存正确性与管理 | 完整 Map 提示词键、大小/TTL、GUI/CLI 清理 |
| 8. 模型分档路由 | 默认、每工具、Discovery、Map 配置及 API 参数兼容 |
| 9. 可验证诊断 | 真实推理、stdio initialize/list/ping、客户端注册单列 |
| 10. 可解释的指标 | 输入/回传估算、服务端 usage、耗时、定价覆盖，不虚报主模型节省 |
| 11. 可操作任务历史 | 自动刷新、筛选、查看/复制/导出、取消、重试 |

Automated regression coverage is described in [DEVELOPMENT.md](DEVELOPMENT.md). Native desktop and individual coding-agent host verification still require those actual environments.

## Retrieval and continuation tools (0.4.0)

`result_page` reads complete successful Job artifacts without model usage and without creating another business Job. `continue_scan` uses the existing `document_repo` route/execution mode and creates the next bounded scan Job. Supplemental analysis passes contribute all actual provider attempts to existing usage/cost metrics. Job `return_tokens` measures the normal initial return; later pagination output is separate. See [CONTINUATIONS.md](CONTINUATIONS.md).
