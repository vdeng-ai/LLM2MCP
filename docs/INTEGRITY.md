# Complete evidence and safe outputs

## Repository documentation

`document_repo` reads eligible files completely and divides each file into numbered line segments before grouping segments into bounded Map requests. A file larger than the per-file budget is continued in later segments rather than dropping its tail. Very long lines are divided without losing UTF-8 characters.

The returned result begins with deterministic scan coverage: complete/eligible files, segment count, and omitted evidence. The operation still has a bounded chunk count and skips binary files and files over 8 MiB. When the bound is reached, coverage identifies the omitted file/range. Narrow `paths`/`include` or run separate requests for omitted modules before relying on repository-wide conclusions.

The scanner remains read-only and applies the existing sensitive-path filters and credential masking before sending segments to a model.
