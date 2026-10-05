LLM2MCP 0.3.0 improves context integrity, evidence quality and verification for secondary coding models.

- Scan complete large files in bounded Map segments; report coverage and omissions.
- Generate validated documentation fragment edits with original SHA-256 hashes, preserving unseen sections.
- Reject length-limited final messages, malformed JSON and unfinished document blocks; recover within three accounted attempts.
- Allocate source, Discovery, Map and synthesis budgets from each routed model's limits.
- Review large diffs in batches with surrounding source, deduplicate findings and prioritize severity; untracked files require explicit opt-in.
- Rerank ambiguous discovery, expose dependency hints and remove unsupported finding citations while marking unverified claims.
- Bind GUI API editing, model refresh/selection and inference tests to the selected named profile.
- Add paired evaluation of actual primary usage, secondary usage, time and externally scored correctness; track provider usage coverage.
- Add opt-in `doctor --deep --workspace PATH --path FILE` and end-to-end coverage for Tasks, active cancellation, portable results and reconnection.

**Compatibility:** `update_docs` now returns `llm2mcp-document-edits-v1` exact edits rather than reconstructed replacement files. Clients applying edits must verify the current file hash, unique old fragment and nonoverlapping edits before writing. The MCP remains read-only. Documentation synthesis requires complete document delimiters.

**Validation:** 58 Rust unit tests, 14 MCP end-to-end tests, 2 paired-evaluation tests, format and Clippy. Release publication is gated on Linux, Windows and macOS CI. Real private-model effectiveness and actual-host compatibility are explicitly separate operator checks; no benchmark saving is claimed.

Linux AppImage and DEB, Windows installer, Apple Silicon/Intel macOS packages, updater signatures, `latest.json` and `SHA256SUMS` are published together.

---

0.3.0 完成九项优化：完整大文件扫描、安全文档局部更新、输出完整性校验与有限恢复、按路由模型分配预算、分批 diff 审查、模糊检索重排与引用检查、命名配置交互修复、实际 usage 成对评测，以及 MCP Tasks/取消/重连的诊断与回归测试。

升级注意：`update_docs` 改为携带原文 SHA-256 的精确片段修改。应用修改前必须核对当前哈希、唯一旧片段和修改不重叠，保留未显示的原文。MCP 仍不直接写文件。真实模型效果评测和真实 Host 验证需按文档在使用环境运行；本版本不宣称未经实测的节省比例。
