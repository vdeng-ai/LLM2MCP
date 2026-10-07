# 0.4.2 — Compiled matching

- Compile include/exclude rules once per source traversal and reuse them across
  task-local discovery scopes. Ordinary rules use `globset`; recursive rules
  use a compiled byte-regex set to preserve the old, broader `**` behavior.
  Basename fallback, root-relative `./` rules, literal brackets/braces,
  byte-oriented `?`, case sensitivity and exclusion priority are unchanged.
- Reuse one keyword matcher per query across discovery, supplemental retrieval
  and document previews. Queries with at least 64 keywords use `aho-corasick`
  with overlapping matches and reusable scratch state. Small queries retain native substring
  search. Build failures fall back to native search.
- Count each keyword once per scored text, preserving overlap, repetition,
  Unicode lowercasing and document-preview weights. Local MCP routing retains
  its existing ASCII-only lowercasing. ASCII automaton searches avoid making
  a lowercase copy; non-ASCII Unicode searches still lowercase the entire text.
- Compute symbol scores once before stable sorting, retaining result ordering
  and the earlier-line tie break for supplemental evidence.

Both matching crates were already in the dependency lockfile; this release
makes them direct dependencies without upgrading them. There are no request
schema or configuration changes.

Compatibility tests compare the compiled filters and scores with the previous
implementation, including Chinese paths/text, generated patterns, overlapping
and duplicate words, empty patterns and scratch-state rollover. Existing MCP
filtering, late evidence, documentation edits, budgets and cancellation
regressions remain required. See the reproducible
[matching benchmark](MATCHING_BENCHMARK.md) for local timing and setup costs;
use [paired evaluation](EVALUATION.md) for actual model latency, correctness,
tokens and cost.
