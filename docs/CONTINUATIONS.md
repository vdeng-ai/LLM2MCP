# Supplemental retrieval, result pages and scan continuations

These v0.4.0 tools remain read-only. They read the configured workspace and store
job artifacts/checkpoints in LLM2MCP's application-data directory.

## One supplemental retrieval round

`analyze`, `debug_issue` and `plan` accept `allow_supplement` (default `true`). The
secondary model can add this optional field alongside its required result fields:

```json
{
  "context_requests": [
    {"query": "worker_timeout_secs", "paths": ["src/worker.rs"]}
  ]
}
```

The bridge handles at most three requests and six new candidates. It selects real
indexed symbols/files locally; literal text fallback looks near matching lines
when symbol labels do not answer the request. The fallback examines up to 32
filtered index candidates per request and 64 MiB in total. Building/refreshing the
existing symbol index retains its usual repository-index cost. Secret paths,
workspace boundaries, `include`/`exclude`, masking and source/model budgets still
apply. Requested paths are hints, not permission to escape those filters.

New evidence is allocated up to half the available source budget, with remaining
space retaining original evidence. The model then gets one additional analysis
pass. Requests in that final pass are reported as unresolved and never trigger
another retrieval round. Empty/invalid/filtered requests do not cause another
analysis pass. Existing bounded provider/schema recovery still applies inside
each logical pass; all actual attempts contribute to Job usage and cost metrics.

```json
{"task":"Explain worker startup failures","paths":["src/jobs.rs"],"allow_supplement":false}
```

Use `false` to retain a single final-analysis pass after normal discovery.
Supplemental retrieval currently covers analysis, debug and planning; batched
review keeps its existing evidence collection.

## Complete-result pagination

Every successful business Job stores its validated, masked complete result before
primary compaction. Compacted analysis/debug/plan/review results contain a
budgeted `FULL_RESULT job_id=... (result_page)` handle. Sync replies also include
`_meta["llm2mcp/job_id"]`; async Job/Task handles already expose the Job ID.
`job_result` continues returning the normal result. For an async Job, you can read pages directly after completion without first fetching its full `job_result`. To inspect all findings:

```json
{"job_id":"job_...","page_tokens":1000}
```

Call `result_page` with those arguments. Its text is JSON:

```json
{
  "format":"llm2mcp-result-page-v1",
  "job_id":"job_...",
  "sha256":"...",
  "total_bytes":24000,
  "offset":0,
  "end":1800,
  "text":"...exact first fragment...",
  "next_cursor":"...",
  "complete":false
}
```

Pass `next_cursor` back as `cursor` with the same Job ID. Concatenate `text` in
`offset` order until `complete: true`, verify byte length/SHA-256 if needed, then
parse JSON or extract complete document blocks. Offsets count UTF-8 bytes; pages
never split a character. A page can split a JSON field or Markdown block, so an
individual page is not an independently applicable document/edit.

`page_tokens` must be an integer from 256 to 8192; the local estimate includes the
JSON page envelope and escaping. Default: 1000. Cursors bind to the Job ID and
content hash. Retrieval checks the workspace, completed state, TTL, byte boundary
and artifact hash. Pages are deterministic and can be reread after reconnection;
there are no model calls. Complete results retain every finding and detail, with
`read_next` represented as locally validated formatted references.

Artifacts are limited to 16 MiB and are removed with expired Job history. Jobs
created before v0.4.0 have no complete-result artifact and must be rerun to use
this tool. Pagination does not change the Job's model usage; `return_tokens`
continues measuring its normal initial return, not subsequent page reads.
Documentation and document-edit jobs retain their complete normal output formats.

## Continue a repository documentation scan

Start a normal `document_repo` request. `max_chunks` (1–12, default 12) limits each
call's Map work, rather than the lifetime of the scan:

```json
{"document_type":"architecture","paths":["src"],"max_chunks":12,"language":"english"}
```

The result includes `SCAN_CONTINUATION` followed by one JSON line:

```json
{
  "scan_cursor":"...",
  "complete":false,
  "files_complete":42,
  "eligible_files":310,
  "segments":118,
  "synthesis_truncated":false,
  "omitted_count":0
}
```

Continue using the `continue_scan` tool:

```json
{"scan_cursor":"..."}
```

It uses the original paths/filters/document settings and existing
`document_repo` profile/execution configuration. Async mode returns another
Job/Task; poll normally. The next result contains the next cursor. You can also
pass `scan_cursor` to `document_repo`; supplied options must match the original
request. The cursor persists across MCP processes and carries the next exact
file/segment, cumulative coverage and Map-summary keys. Replaying a cursor
replays that page, with cached Maps reused; synthesis is a new accounted pass.

Continue until `scan_cursor: null` and `complete: true`. This means the selected
scan iterator is exhausted. Compare `files_complete` with `eligible_files` and
inspect omitted evidence: binary/oversized/unreadable files and segments too
large for their budget can still be skipped. Intermediate documentation is
provisional. Every synthesis uses accumulated Map summaries, not only the latest
page; a large accumulated map can still exceed the Reduce budget, reported by
`synthesis_truncated`. A complete scan is not a guarantee that every summary fits
in one synthesis request.

Snapshots check the eligible path list and file size/nanosecond-mtime stamps,
plus the content hash of an in-progress file. Scope, Map model/profile/prompt or
segment-budget changes require a fresh scan. Changes during Map/Reduce are also
checked before returning a continuation. Cursor files are immutable and
workspace-bound. Checkpoints and their Maps share configured cache size/TTL and
manual cleanup; an expired/evicted checkpoint or Map requires a new scan.
Checkpoints are limited to 16 MiB. Cancellation/failure never advances a returned
cursor; retrying the same request reuses successfully cached Maps.

---

# 中文操作要点

- **补充检索：** `analyze/debug_issue/plan` 默认允许模型提出一次补充检索。最多 3 个请求、6 个新候选；优先真实 symbol，必要时从已过滤的索引候选中做有界文本匹配，读取匹配行附近内容。新增与原有源码共用预算。`allow_supplement: false` 可关闭。最终分析再次提出的请求会标记未解决，不会循环调用。
- **完整结果：** 从回传的 `FULL_RESULT`、Sync `_meta` 或后台 Job/Task 取得 `job_id`，调用 `result_page`，反复传回 `next_cursor`。按字节偏移拼接 `text`，直至 `complete: true`，再解析 JSON 或应用完整文档。分页本身不调用模型，支持断线重连，保留全部发现和细节；源码引用与 read-next 仍经过本地校验。
- **大仓库续扫：** `document_repo` 每次处理最多 12 个 chunk，返回 `SCAN_CONTINUATION`。调用 `continue_scan` 并传入 `scan_cursor`，直到游标为空。每次合成使用累计 Map 证据；`complete` 表示扫描遍历结束，还需查看遗漏数量和 `synthesis_truncated`。源码、过滤范围、Map 配置或分段预算变化时应重新开始。
- **使用限制：** 分页每页 256–8192 token（本地估算，含 JSON 包装）；完整结果与检查点各不超过 16 MiB。结果跟随 Job TTL 清理，续扫状态与 Map 跟随缓存大小/TTL 清理。每个逻辑分析阶段仍可能因 Provider/schema 错误做有限重试，实际调用和费用均记录。
