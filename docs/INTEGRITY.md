# Complete evidence and safe outputs

## Repository documentation

`document_repo` reads eligible files completely and divides each file into numbered line segments before grouping segments into bounded Map requests. A file larger than the per-file budget is continued in later segments rather than dropping its tail. Very long lines are divided without losing UTF-8 characters.

The returned result begins with deterministic scan coverage: complete/eligible files, segment count, and omitted evidence. The operation still has a bounded chunk count and skips binary files and files over 8 MiB. When the bound is reached, coverage identifies the omitted file/range. Narrow `paths`/`include` or run separate requests for omitted modules before relying on repository-wide conclusions.

The scanner remains read-only and applies the existing sensitive-path filters and credential masking before sending segments to a model.

## Documentation updates

`update_docs` returns JSON in `llm2mcp-document-edits-v1` format. Each edit contains `path`, `original_sha256`, a unique exact `old_text` fragment from the supplied preview, and `new_text`. The bridge checks the full original locally, rejects stale hashes, ambiguous matches, overlapping edits and invented paths, and never writes files. Preview limits do not remove unseen sections because the result is a local edit, not a reconstructed whole document.

The primary agent must check the file's current SHA-256 before applying the edits, apply each old fragment exactly once, and preserve all remaining text. If the file changed, request a fresh update. `new_documents` can contain complete new `docs/*.md` files whose paths do not exist; their parent paths are sandbox checked. Truncated previews/diffs and omitted documents are returned explicitly. Up to 64 document previews are considered per request; use explicit `docs` to update later documents or sections.

## Output completeness

A nonempty response with `finish_reason=length` or `content_filter` is never accepted as final. Structured tools require their documented JSON fields; documentation synthesis requires every requested document boundary and a closing marker. The bridge makes at most three attempts, lowers reasoning when supported, and grows the output allowance within the selected model's context/output ceiling. Every attempt contributes to recorded usage and cost. Exhaustion reports failure instead of returning partial findings or Markdown.

## Routed context budgets

Each tool derives source limits from its routed model ceiling after reserving its system prefix, instructions, request metadata, manifest and final output. Discovery and Map derive their own limits before collecting evidence. Small models cap output reservations and source segment sizes; synthesis allocates space fairly across all Map summaries and marks shortened summaries. Provider preflight still rejects any request that exceeds the configured ceiling. Token counts are conservative local estimates, not a provider tokenizer guarantee.

## Diff review coverage

`review_diff` enumerates changed paths and reviews every diff segment in batches with nearby current source, rather than clipping the global diff prefix. Findings are deduplicated and ordered by severity. Up to 32 batches are reviewed per call; omitted paths/parts are explicitly reported. Set `include_untracked: true` to include nonignored new text files; secret paths, detected secret literals and binary/oversized files retain the existing filtering. Deleted-file hunks remain reviewable without current source.

## Discovery and citations

Ambiguous single-word or tied lexical matches are reranked rather than treated as confident symbol selection. Unique explicit identifiers still avoid an unnecessary model call. The index includes locally extracted dependency hints; hints do not prove runtime reachability.

Structured analysis/debug/review finding citations are checked against the source actually supplied, not the repository manifest. Exact numbered ranges must be wholly present; nonexistent or unseen references are removed, and claims without a verified location are visibly marked `UNVERIFIED CLAIM`. This validates source locations, not the truth of the model's reasoning. Whole-file evidence is numbered before transmission.

## Selected profile in the GUI

Choose `Editing / 编辑配置` before editing API URL/key/model or refreshing models. The picker, model list and top-bar inference test operate on that same profile. Fallback fields remain separate, and delayed model-list replies carry their originating profile; lists from another profile are never offered in the current picker. Rename/removal continues to update all profile routes.
