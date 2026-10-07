# LLM2MCP 0.3.1 quality fixes

The patch release improves the quality of the existing read-only tools. It does
not introduce additional model calls or change the document edit format.

| Stage | Change | Acceptance |
| --- | --- | --- |
| 1 | Budget the entire primary response | Review coverage and omissions fit inside the configured return budget. |
| 2 | Preserve important results | Long read-next descriptions or conclusions cannot displace the highest-severity finding. |
| 3 | Check all supplied evidence types | Source ranges, log excerpts and diff hunks are validated against their respective supplied material; unsupported debug claims lower confidence. |
| 4 | Select relevant document fragments | Changes to an API/configuration select the affected Markdown section, including late sections; exact-edit validation still rejects unseen, ambiguous and overlapping replacements. |
| 5 | Regression checks and patch metadata | Format, Rust tests, MCP end-to-end tests, evaluation tests and Clippy pass; document the behavioral changes. |

Each implementation stage is committed separately. Actual private-model and
native-host benchmarks remain operator-run checks; this patch makes no claim of
measured token savings.
