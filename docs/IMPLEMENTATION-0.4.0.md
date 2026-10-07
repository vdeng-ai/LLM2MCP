# LLM2MCP 0.4.0: bounded follow-up and continuations

## Scope

- `analyze`, `debug_issue` and `plan` can request one supplemental local retrieval pass using optional `context_requests`. Up to three requests and six new source candidates are selected locally under existing filters, privacy and source/model budgets. At most one additional final-analysis pass is made; existing bounded provider/schema recovery is accounted; `allow_supplement: false` disables it. No agent loop or command execution is introduced.
- Every successful business job retains its validated complete result before primary compaction. `result_page` reads immutable UTF-8 pages using a job ID and content-bound cursor without model calls. Workspace, terminal state, cursor and page budget are checked; existing Job TTL/cleanup applies.
- Repository documentation scans retain an immutable continuation checkpoint. `continue_scan` resumes the next file/segment and synthesizes accumulated map summaries. Eligible path/stamp, in-progress content, filter/layout/profile changes reject stale cursors; per-call chunks remain bounded and cancellation/retry reuse map caches. Coverage distinguishes pending evidence from skipped evidence.

## Compatibility and acceptance

Existing calls without supplemental requests or continuation arguments keep their normal result formats. Compacted analysis/review returns include a budgeted full-result handle. Documentation edit JSON remains unchanged. Full-result pages are retrieval artifacts, never accepted as complete replacement documents until reconstructed.

Tests must cover one-pass retrieval limits and denial/filtering, Unicode page reconstruction and stale/foreign cursors, reconnect and async paging, scanning beyond one chunk page without duplicates or gaps, changed-repository rejection, privacy, cancellation/cache reuse and all existing regressions. Linux/Windows/macOS CI and format/Clippy gate the development PR.

## Completed local validation

All three features are implemented. Linux local checks pass with Rust 1.99.0:

- `cargo fmt -- --check` and `git diff --check`.
- `cargo test --locked --all-targets`: 72 unit tests and 23 actual-binary MCP end-to-end tests.
- `cargo clippy --locked --all-targets --all-features -- -D warnings`.
- `LLM2MCP_TEST_BINARY=target/debug/llm2mcp python -m unittest discover -s scripts/evaluation -v`: both tests execute, including real HTTP/stdio MCP.

Regressions demonstrate literal matching beyond a source prefix, one-pass retrieval
limits and active cancellation, complete Unicode page reconstruction after sync
and async reconnection, stale/foreign/expired result rejection and temporary
artifact cleanup, source scanning beyond 12 chunks without gaps, cursor replay
without duplicate Maps, cumulative late evidence, and source/options/profile
change rejection. Cross-platform CI gates the development PR.

Version metadata is 0.4.0. The matching `release.json` request starts the repository's
gated release workflow only when merged into `main`; development does not merge
or publish this release automatically. Real private-model and actual-host
verification remain operator checks.
