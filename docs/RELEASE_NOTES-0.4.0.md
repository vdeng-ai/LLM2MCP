LLM2MCP 0.4.0 adds bounded follow-up retrieval and durable continuations.

- `analyze`, `debug_issue` and `plan` accept optional `context_requests` from the secondary model. Retrieve up to six real source candidates from at most three requests, with indexed-symbol and bounded literal-text matching, and perform one additional analysis pass. Existing privacy, filtering, cancellation and source/model budgets remain enforced; `allow_supplement: false` disables follow-up.
- Preserve each successful business Job's validated complete result before primary compaction. `result_page` retrieves exact UTF-8 pages without model calls, using Job/content-bound cursors, workspace checks, artifact integrity and existing Job TTL. Compacted returns include a budgeted Job handle; document-edit JSON remains unchanged.
- Replace the repository scan's lifetime chunk ceiling with a per-call ceiling. `continue_scan` resumes an immutable file/segment checkpoint, reuses Map caches and synthesizes accumulated evidence. Coverage reports pending work, skipped evidence and synthesis truncation. Workspace/source/configuration changes reject stale cursors.

**Compatibility:** Existing tool arguments remain valid. Three analysis tools add `allow_supplement`; `document_repo` adds `max_chunks` and `scan_cursor`. MCP now advertises `result_page` and `continue_scan`. `job_result` remains the normal summary/document output. Repository documentation adds a `SCAN_CONTINUATION` metadata line outside document blocks. Full-result references are locally validated formatted `read_next` strings. Reassemble all page text before parsing JSON or applying documents/edits. Jobs predating this version need rerunning for complete-result pagination.

**Limits:** One supplemental logical pass can still use the existing bounded provider/schema recovery; all attempts are counted. Literal fallback reads at most 32 filtered index candidates per request and 64 MiB per retrieval round. Job artifacts and scan checkpoints each have a 16 MiB limit. Continuations require unchanged eligible path/stamp snapshots and in-progress content hashes, and compatible Map profiles/segment layouts. Cache eviction/expiry requires restarting. Accumulated Map evidence is still bounded for Reduce, so a completed scan does not guarantee untruncated synthesis. No measured token-saving or private-model-quality claim is made.

**Validation:** 72 Rust unit tests, 24 MCP end-to-end tests, 2 paired-evaluation tests, format and Clippy pass locally. Linux, Windows and macOS CI gate the development PR. Details are documented in [IMPLEMENTATION-0.4.0.md](IMPLEMENTATION-0.4.0.md). Usage and JSON examples are in [CONTINUATIONS.md](CONTINUATIONS.md).

---

0.4.0 新增三项能力：一次有界补充检索、完整结果分页、大仓库持久化续扫。仍保持只读，不执行 shell 或修改工作区。补充检索与原证据共用预算；分页可跨进程恢复并校验内容；续扫按文件/分段推进、复用 Map 缓存并合成累计证据。源码或相关配置变化时游标失效。分页须拼接完整后再解析或应用；扫描结束仍需查看遗漏与合成截断标记。
