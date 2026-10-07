# Complete evidence and safe outputs

## Repository documentation

`document_repo` reads eligible files completely and divides each file into numbered line segments before grouping segments into bounded Map requests. A file larger than the per-file budget is continued in later segments rather than dropping its tail. Very long lines are divided without losing UTF-8 characters.

The returned result begins with deterministic scan coverage: complete/eligible files, segment count, and omitted evidence. Each call still has a bounded chunk count and skips binary files and files over 8 MiB. When the per-call bound is reached, `SCAN_CONTINUATION` identifies pending work and provides a persistent `scan_cursor`. `continue_scan` resumes the next exact file/segment and reuses accumulated Maps; it validates eligible path/stamp snapshots, in-progress content hashes, workspace and Map layout. Skipped binary/oversized/unreadable evidence remains explicit. Final Reduce still has a context budget and reports `synthesis_truncated`. Narrow `paths`/`include` when accumulated Maps cannot fit. See [continuation usage](CONTINUATIONS.md).

The scanner remains read-only and applies the existing sensitive-path filters and credential masking before sending segments to a model.

## Documentation updates

`update_docs` returns JSON in `llm2mcp-document-edits-v1` format. Each edit contains `path`, `original_sha256`, a unique exact `old_text` fragment entirely within one supplied `preview_fragments` item, and `new_text`. The bridge checks the full original locally, rejects stale hashes, ambiguous matches, overlapping edits and invented paths, and never writes files. Preview limits do not remove unseen sections because the result is a local edit, not a reconstructed whole document.

Long documents are selected locally using changed identifiers and Markdown ATX sections, including late sections. Up to four matching sections share the preview budget; large sections retain the heading and context near the matching line. Unmatched documents fall back to a bounded prefix. Fragments contain exact masked source text with original line ranges, and separate fragments cannot be joined into one edit. This is lexical selection, so prose without matching identifiers can still require narrower explicit document requests.

The primary agent must check the file's current SHA-256 before applying the edits, apply each old fragment exactly once, and preserve all remaining text. If the file changed, request a fresh update. `new_documents` can contain complete new `docs/*.md` files whose paths do not exist; their parent paths are sandbox checked. Truncated previews/diffs and omitted documents are returned explicitly. Up to 64 document previews are considered per request; use explicit `docs` to update later documents or sections.

## Output completeness

A nonempty response with `finish_reason=length` or `content_filter` is never accepted as final. Structured tools require their documented JSON fields; documentation synthesis requires every requested document boundary and a closing marker. The bridge makes at most three attempts, lowers reasoning when supported, and grows the output allowance within the selected model's context/output ceiling. Every attempt contributes to recorded usage and cost. Exhaustion reports failure instead of returning partial findings or Markdown.

## Routed context budgets

Each tool derives source limits from its routed model ceiling after reserving its system prefix, instructions, request metadata, manifest and final output. Discovery and Map derive their own limits before collecting evidence. Small models cap output reservations and source segment sizes; synthesis allocates space fairly across all Map summaries and marks shortened summaries. Provider preflight still rejects any request that exceeds the configured ceiling. Token counts are conservative local estimates, not a provider tokenizer guarantee.

## Supplemental retrieval and complete results

Analysis, debug and planning allow one local supplemental round on model request (up to three requests/six new candidates), with indexed symbols and bounded literal text matching. Existing source filters, masking, cancellation and budgets apply; new evidence shares the source budget with original evidence. Unresolved final requests do not loop. Provider/schema retries remain bounded and accounted.

Successful Jobs retain a validated, masked complete-result artifact before primary compaction. `result_page` reads immutable Job/content-bound UTF-8 pages without model calls; it checks workspace, completion, TTL, offsets and artifact hash. Page envelopes/escaping count toward the page budget. Complete artifacts are limited to 16 MiB and cleaned with Job history. Reassemble pages before parsing JSON or applying documents. Existing `job_result` stays compatible.

## Primary return budget

Analysis, debug, planning and review results are compacted locally without an extra model call. The complete review return, including coverage and omissions, fits the configured primary return budget. Long omission lists are explicitly shortened. Compacted analysis/debug/plan/review returns also reserve space for a complete-result Job handle. Findings are ranked by severity before item limits; present summary, findings, READ_NEXT and detail groups reserve weighted shares instead of allowing one verbose field to consume the result. Short complete results are preserved, and shortened results carry a truncation marker. Limits use the local conservative estimator rather than exact provider tokenization.

## Diff review coverage

`review_diff` enumerates changed paths and reviews every diff segment in batches with nearby current source, rather than clipping the global diff prefix. Findings are deduplicated and ordered by severity. Up to 32 batches are reviewed per call; omitted paths/parts are explicitly reported. Set `include_untracked: true` to include nonignored new text files; secret paths, detected secret literals and binary/oversized files retain the existing filtering. Deleted-file hunks remain reviewable without current source.

## Discovery and citations

Ambiguous single-word or tied lexical matches are reranked rather than treated as confident symbol selection. Unique explicit identifiers still avoid an unnecessary model call. The index includes locally extracted dependency hints; hints do not prove runtime reachability.

Structured analysis/debug/review finding citations are checked against dedicated catalogs of the actual supplied source, logs and patch bodies, rather than instructions or the repository manifest. Use `path:start-end`, `log:exact excerpt`, or `diff:path:exact hunk/excerpt`. Runtime excerpts must contain at least eight characters and occur in the appropriate supplied material; legacy whole-file references and exact runtime excerpts remain accepted. Exact numbered ranges must be wholly present; nonexistent or unseen references are removed, and claims without a verified location are visibly marked `UNVERIFIED CLAIM`. This validates source locations, not the truth of the model's reasoning. Whole-file evidence is numbered before transmission. Debug confidence is capped at low when no finding has a valid citation, and high becomes medium when citations are partially unsupported. A valid citation does not establish that the diagnosis is correct.

## Selected profile in the GUI

Choose `Editing / 编辑配置` before editing API URL/key/model or refreshing models. The picker, model list and top-bar inference test operate on that same profile. Fallback fields remain separate, and delayed model-list replies carry their originating profile; lists from another profile are never offered in the current picker. Rename/removal continues to update all profile routes.
