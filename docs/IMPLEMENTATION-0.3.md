# LLM2MCP 0.3 implementation plan

Each numbered stage is implemented and committed separately. Existing workspace access remains read-only. Release publication is the last stage, after regression checks and cross-platform CI.

| Stage | Scope | Acceptance |
| --- | --- | --- |
| 1 | Complete large-file repository scanning | Later source ranges appear in Map chunks; source coverage and omissions are explicit. |
| 2 | Safe documentation updates | A truncated original never becomes an unchecked complete replacement; edits retain source hashes and unchanged sections. |
| 3 | Complete model outputs | Length-limited and malformed structured outputs trigger bounded recovery or a clear failure; document boundaries are checked. |
| 4 | Profile-aware budgets | Discovery, Map, analysis and synthesis use their own model ceiling minus prompt/output overhead. |
| 5 | Complete diff review | Large changes are reviewed in batches, relevant surrounding source is included, and untracked files require opt-in. |
| 6 | Discovery and evidence quality | Ambiguous single-word matches get reranked; dependency hints and all returned citations are checked against supplied source. |
| 7 | Named-profile interaction | Model refresh, selection, editing and inference tests operate on the same selected profile. |
| 8 | Reproducible effectiveness evaluation | Paired runs record actual primary usage, secondary usage, time and externally scored correctness without treating compression as verified savings. |
| 9 | MCP/host verification | Diagnostics exercise tool calls, portable jobs and modern Tasks/cancellation/reconnection; actual-host checks are explicitly separate. |
| 10 | Release 0.3.0 | Format, tests and Clippy pass locally; Linux/Windows/macOS CI passes; signed artifacts, checksums and updater manifest are published. |

Tests focus on observable regressions: late-file evidence, original-document preservation, length-limited answers, small routed profiles, late diff findings, ambiguous search, profile selection and protocol lifecycle.

Real-model evaluation is opt-in and uses the operator's configured APIs. A reproducible evaluation runner can be delivered without access to a private inference endpoint; real measurements must never be invented.

## Commit sequence

1. `feat: scan complete source files in bounded repository chunks`
2. `fix: preserve original documents when generating updates`
3. `fix: validate and recover incomplete model outputs`
4. `feat: allocate context budgets per routed model`
5. `feat: review bounded diff batches with source context`
6. `feat: verify analysis evidence and rerank ambiguous discovery`
7. `fix: bind API interactions to the selected model profile`
8. `feat: add paired effectiveness evaluation tooling`
9. `feat: verify MCP tool and task lifecycles in diagnostics`
10. `release: publish LLM2MCP 0.3.0`

## Implementation status

Stages 1–9 are implemented in sequential commits. A subsequent integration fix tightens nested output schemas and cleans synthetic diagnostic handles on failure. The release change runs the final local suite and reusable cross-platform CI before packaging/publishing. Local validation: 58 Rust unit tests, 14 MCP end-to-end tests, 2 paired-evaluation tests, formatting and warning-free Clippy. Private real-model and actual-host evaluation remain operator-run checks, documented in EVALUATION.md and DIAGNOSTICS.md.
