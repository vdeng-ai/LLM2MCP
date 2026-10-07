LLM2MCP 0.3.1 improves the quality of existing read-only tools.

- Count review coverage and omission lists inside the complete primary return budget. Long omission lists carry an explicit truncation marker.
- Sort findings by severity before item limits and reserve separate space for summaries, findings, validated READ_NEXT entries and details during local compaction.
- Validate source ranges, log excerpts and diff excerpts against the actual supplied material. Remove unsupported citations, mark unverified claims and cap debug confidence when evidence is missing or partial. Handle Git quoted paths and header-like text within hunks.
- Select relevant Markdown sections near changed identifiers, including sections late in long documents. Preserve exact text, original hashes, unique-match and nonoverlap checks; reject edits crossing separate preview fragments.

**Compatibility:** MCP tool arguments and the `llm2mcp-document-edits-v1` response format remain unchanged. The internal model prompt now supplies `preview_fragments` with line ranges instead of a single document prefix. Compacted results place summaries/findings before READ_NEXT while reserving space for each group. No extra model calls or workspace writes are introduced.

**Limits:** Return limits use conservative local token estimates. Citation validation checks supplied locations/excerpts, not whether model reasoning is true. Documentation selection is lexical and uses ATX Markdown headings; prose without matching identifiers falls back to a bounded prefix. Real private-model effectiveness and native-host verification remain separate operator checks; this patch makes no measured token-saving claim.

**Validation:** 68 Rust unit tests, 16 MCP end-to-end tests and 2 paired-evaluation HTTP/MCP tests pass locally, together with format and Clippy (`-D warnings`). Linux, Windows and macOS CI gates apply before release publication.

---

0.3.1 修复四项质量问题：覆盖说明计入完整回传预算；严重问题在压缩前排序并保留独立预算；源码、日志和 diff 引用按实际证据校验，缺少或部分不支持的 Debug 证据限制置信度；文档更新按变更标识符选择相关章节，并保持精确片段修改的哈希、唯一匹配和不重叠校验。

MCP 参数、文档修改返回格式和只读行为保持不变，不增加模型调用。文档选择仍是词法匹配；真实模型效果与 Host 集成需在使用环境验证。
