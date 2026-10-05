# Complete evidence and safe outputs

## Repository documentation

`document_repo` reads eligible files completely and divides each file into numbered line segments before grouping segments into bounded Map requests. A file larger than the per-file budget is continued in later segments rather than dropping its tail. Very long lines are divided without losing UTF-8 characters.

The returned result begins with deterministic scan coverage: complete/eligible files, segment count, and omitted evidence. The operation still has a bounded chunk count and skips binary files and files over 8 MiB. When the bound is reached, coverage identifies the omitted file/range. Narrow `paths`/`include` or run separate requests for omitted modules before relying on repository-wide conclusions.

The scanner remains read-only and applies the existing sensitive-path filters and credential masking before sending segments to a model.

## Documentation updates

`update_docs` returns JSON in `llm2mcp-document-edits-v1` format. Each edit contains `path`, `original_sha256`, a unique exact `old_text` fragment from the supplied preview, and `new_text`. The bridge checks the full original locally, rejects stale hashes, ambiguous matches, overlapping edits and invented paths, and never writes files. Preview limits do not remove unseen sections because the result is a local edit, not a reconstructed whole document.

The primary agent must check the file's current SHA-256 before applying the edits, apply each old fragment exactly once, and preserve all remaining text. If the file changed, request a fresh update. `new_documents` can contain complete new `docs/*.md` files whose paths do not exist; their parent paths are sandbox checked. Truncated previews/diffs and omitted documents are returned explicitly. Up to 64 document previews are considered per request; use explicit `docs` to update later documents or sections.
