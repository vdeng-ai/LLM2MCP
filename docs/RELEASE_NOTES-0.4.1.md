# 0.4.1 — Runtime efficiency

- Repository Map uses a bounded worker queue instead of batches. A finished
  worker takes the next pending chunk immediately; completion progress is
  reported as it arrives, while Map evidence retains source order. Existing
  worker/HTTP limits, cancellation and content-addressed caches still apply.
- `document_repo` accepts `synthesis: "final"` to defer document generation until
  the scan completes. Intermediate pages persist Maps/cursors and report
  `synthesis_deferred: true`. The default `each_page` preserves existing behavior.
- Analyze/debug/plan reuse a task-local discovery snapshot across supplemental
  queries, avoiding repeated scope walks, symbol-index loads and file indexing.
  New broader scopes are loaded lazily. Exact source evidence still validates
  current file stamps and symbol ranges. Newly added candidate files appear on
  the next task; task-local reuse contributes to symbol cache hits.
- Automatic cache maintenance uses a shared five-minute timestamp and a
  nonblocking cross-process lock. Changed size/TTL policy, invalid timestamps
  and clock rollback trigger a fresh pass. Explicit `cache` / `cache --clear`
  commands run immediately. Cache size/TTL enforcement can lag by five minutes.
- HTTP calls share one two-thread Tokio runtime and a reqwest connection pool
  per process. Request headers/keys, deadlines, cancellation and provider retries
  remain independent; there is no cross-process connection pooling.

See [continuation usage](CONTINUATIONS.md) for the new request option. Intermediate
`final` scan Jobs complete with progress, not documentation. Final synthesis
still uses bounded accumulated Map evidence and reports truncation; this release
does not add hierarchical summary reduction or change reasoning defaults.

Validation covers work-conserving scheduling, ordered output, failure handling,
HTTP keep-alive across callers, independent timeout/authentication, stale symbol
rejection, final-only synthesis including replay/reconnection and async Jobs,
and cross-process cache throttling with immediate manual pruning. Existing MCP
lifecycle, cancellation, filtering, privacy, budgets and evaluation regressions
remain required. No real-model benchmark or percentage saving is claimed; use
[paired evaluation](EVALUATION.md) for model-specific speed, correctness, tokens
and cost measurements.
