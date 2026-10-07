# LLM2MCP Development Guide

This document contains development, local build, test, and packaging notes. The project README stays focused on product features and usage.

## Local development

Run the GUI from the repository root:

```bash
cargo run
```

Refresh the stable per-user executable and print its installed path:

```bash
cargo run -- install
```

Run the stdio MCP server directly against a workspace:

```bash
cargo run -- mcp --workspace /path/to/project
```

In stdio mode, stdout is reserved for MCP JSON-RPC. Runtime diagnostics must go to stderr.

## Recommended local checks

Before committing changes:

```bash
cargo fmt -- --check
cargo test --locked --all-targets
cargo clippy --all-targets --all-features -- -D warnings
```

The GitHub Actions CI runs tests on Linux, Windows, and macOS, with format and Clippy checks on Linux.

## GUI development notes

The GUI uses `eframe`/`egui` and is organized as six module tabs:

- LLM API
- Context limits
- Per-tool reasoning
- Global System Prompt prefix
- Background job history
- AI coding agents

Native window size/position, UI zoom, and the last active tab are persisted by the desktop application.

## Stable executable behavior

Coding-agent integrations are registered against a stable per-user LLM2MCP executable instead of a transient `target/debug/...` path. This keeps MCP integrations working across normal development rebuilds.

To refresh that stable copy manually:

```bash
cargo run -- install
```

## Packaging and releases

Release packaging, updater signing, GitHub Actions tag releases, Windows NSIS, macOS app/DMG bundles, Linux AppImage, and automatic-update setup are documented separately in:

- [RELEASE.md](RELEASE.md)

Architecture and roadmap:

- [ARCHITECTURE.md](ARCHITECTURE.md)
- [ROADMAP.md](ROADMAP.md)

---

# 开发说明

本文档集中记录开发、构建、测试和发布相关内容，项目 README 主要保留产品特点与使用方法。

## 本地开发

在仓库根目录启动 GUI：

```bash
cargo run
```

刷新稳定的用户级可执行文件并输出其路径：

```bash
cargo run -- install
```

直接针对某个项目启动 stdio MCP：

```bash
cargo run -- mcp --workspace /path/to/project
```

stdio 模式下 stdout 仅用于 MCP JSON-RPC，运行诊断信息必须写入 stderr。

## 提交前检查

建议执行：

```bash
cargo fmt -- --check
cargo test --locked --all-targets
cargo clippy --all-targets --all-features -- -D warnings
```

GitHub Actions 会在 Linux、Windows、macOS 上运行测试，并在 Linux 上额外执行格式检查和 Clippy。

## GUI 开发说明

GUI 基于 `eframe`/`egui`，当前分为六个一级标签页：

- LLM API
- 上下文限制
- 工具独立思考强度
- 全局 System Prompt 前缀
- 后台 Job 历史
- AI 编码智能体

桌面程序会持久化窗口位置/尺寸、UI 缩放比例和最后打开的标签页。

## 稳定可执行文件

编码智能体集成指向稳定的用户级 LLM2MCP 可执行文件，而不是临时 `target/debug/...` 路径，因此普通开发重编译不会让已注册的 MCP 路径失效。

手工刷新稳定副本：

```bash
cargo run -- install
```

## 打包与发布

Release 打包、Updater 签名、GitHub Actions tag 发布、Windows NSIS、macOS app/DMG、Linux AppImage 和自动更新配置统一见：

- [RELEASE.md](RELEASE.md)

其它开发文档：

- [ARCHITECTURE.md](ARCHITECTURE.md)
- [ROADMAP.md](ROADMAP.md)

## Runtime regression tests

`tests/mcp_e2e.rs` starts the actual executable with an isolated config/data directory and a loopback mock HTTP service. It checks that slow HTTP does not block ping, request cancellation terminates sync history, 429 retry preserves payload/privacy, profile routing and usage/cost metrics are recorded, `/models`-only connectivity fails doctor, worker crashes are recovered/retried, queued workers respect concurrency, Map/Reduce route and prefix changes invalidate Map checkpoints, oversized requests are rejected before HTTP, and cache age/size cleanup preserves history. No real API credentials or host installs are used.

The 0.3.1 regressions cover review coverage inside the return budget, late critical findings during compaction, source/log/diff citation separation and confidence limits, Git quoted paths and header-like hunk text, relevant late Markdown sections, Unicode/CRLF previews, and rejection of edits crossing separate fragments. MCP tests exercise runtime evidence validation and exact document edits without writing workspace files.

Unit tests cover credential masking with quote/line preservation, AST multiline and late-file ranges, Chinese intent matching, relevant/dependency files before budget exhaustion, tracked secret-file diff exclusion, source/evidence masking, cache prompt-key invalidation, profile migration and endpoint-bound credential restoration.

For a native desktop check, run `cargo run`, start a slow model-list/inference operation and verify the window remains interactive and Cancel works. Verify all six tabs, profile rename/delete routing, history auto-refresh/result copy and client registration in the actual host. CI compilation does not establish host integration or desktop rendering.

CLI usage and metric definitions are in [RUNTIME.md](RUNTIME.md).

## 0.4.0 regression coverage

Unit tests cover UTF-8 page reconstruction including JSON escaping, page budgets and cursor integrity, literal source matches beyond a file prefix, bounded read work, and source scans beyond 12 chunks without gaps. MCP end-to-end tests cover supplemental limits/disable/filtering/cancellation, full-result reconstruction after sync and async reconnection, invalid inputs, foreign workspaces, artifact mutation/expiry/cleanup, continuation replay without duplicate Maps, accumulated late evidence, and stale source/scope/profile rejection. Paired evaluation is run with `LLM2MCP_TEST_BINARY` set so both HTTP and actual stdio MCP paths execute.
