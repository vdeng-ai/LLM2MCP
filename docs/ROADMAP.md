# LLM2MCP 开发计划

## 0.1 — 可用 MVP

- [x] Rust 单二进制骨架
- [x] egui 配置界面
- [x] English / 简体中文界面
- [x] 跨平台系统 CJK 字体 fallback
- [x] OpenAI-compatible Chat Completions
- [x] 多 reasoning 协议
- [x] Analyze / Plan / Review Diff / Document Repo / Update Docs 独立思考强度、输出预算与 Sync/Auto/Async 执行策略
- [x] 本地 workspace 沙箱
- [x] `.gitignore` aware 文件扫描
- [x] 常见敏感文件排除
- [x] stdio MCP 基础协议
- [x] Cursor 用户级 MCP 配置合并/卸载
- [x] Claude Code 官方 MCP CLI 集成
- [x] Codex 官方 MCP CLI 集成
- [x] Grok Build 官方 MCP CLI 集成
- [x] Pi MCP extension + 用户级配置集成
- [x] 其他 stdio MCP Client 通用启动方式
- [x] MCP 不传 workspace 时默认使用调用方当前工作目录
- [x] 完成 Linux 编译、单元测试和 Clippy
- [x] 验证 stdio MCP initialize / tools/list / tools/call
- [x] 用远程 LiteLLM + Qwen 做端到端 smoke test
- [x] `document_repo` 分块扫描 + Repository Map→Reduce 文档合成
- [x] Repository Map compact summary（约 800–1200 tokens）+ 可配置并发（默认 2）
- [x] 内容寻址 Repository Map checkpoint/cache，跨 Job 复用未变化 chunk
- [x] Reduce 空 final 诊断与 reasoning 自动降级重试（XHigh → Medium → Low）
- [x] Reduce 最终失败正确记录 `state=failed / stage=reduce_failed`，不再误报 completed
- [x] `update_docs` Git diff + 现有文档增量更新
- [x] Durable Job Manager：持久化 Job 状态、结果与 worker 日志
- [x] 独立 `job-worker` 子进程，后台任务生命周期与原 MCP 请求解耦
- [x] `job_status / job_result / job_cancel` 通用兼容工具
- [x] `document_repo / update_docs` 默认 Async；`analyze / plan` 默认 Auto
- [x] Job stage/progress 持久化与跨 MCP 进程恢复
- [x] 跨进程 Job 文件锁 + Unix 原子状态替换
- [x] MCP `2026-07-28` `server/discover` 现代协议兼容
- [x] `io.modelcontextprotocol/tasks` Tasks Extension：task handle / `tasks/get` / `tasks/update` / `tasks/cancel`
- [x] Tasks capability per-request 校验和 legacy fallback
- [x] Codex / Claude Code / Grok Build 安装时尝试配置更长 MCP tool timeout
- [x] AI 编码智能体安装统一使用稳定的用户级 LLM2MCP 可执行文件，避免开发构建路径失效
- [x] 直接修改的客户端 JSON/TOML 配置支持 `*.llm2mcp.bak` 备份、权限保留与安全替换
- [x] English README / 中文 README 双语文档
- [ ] Cursor GUI 实机验证 MCP 发现与工具调用
- [ ] Codex / Claude Code / Grok Build 实机 MCP 验证
- [ ] Pi + MCP extension 实机验证
- [ ] Windows 11 构建与多客户端验证
- [ ] macOS 构建验证

## 0.2 — 上下文选择优化

- [x] `analyze/debug_issue/plan` 两阶段代码发现：先根据轻量文件 / symbol 索引选择关键文件，再进行深度分析
- [x] Discovery 优先使用本地 lexical/symbol scoring；匹配明确时跳过额外 LLM rerank
- [x] 深度上下文从文件级进一步收敛到 symbol + 精确 line range，并只携带少量文件头/import 上下文
- [x] 输入上下文使用 token 估算预算，替代仅按字符数截断
- [x] `analyze / plan / document_repo` 支持 include / exclude glob
- [x] `analyze / debug_issue / plan / review_diff` 分离副模型输出预算与主模型回传预算，结果在本地结构化压缩
- [x] `analyze / debug_issue / plan` 返回经过候选索引校验的 `file:symbol:line-range` read-next 证据；虚构范围不会回传
- [x] `debug_issue` 专用故障诊断：症状/日志/预期与实际行为 + 可选近期 Git diff → 根因、执行路径、间歇性解释、验证步骤和修复区域；保持只读
- [x] 持久化 Repository Symbol Index：按 workspace + 文件 size/mtime 增量复用，变化文件自动重新哈希/解析
- [x] 持久化 Evidence Cache：按文件内容 SHA-256 + 精确 symbol 行段复用本地源码证据，源码变化自动失效
- [x] Job 历史/status 暴露 Symbol Index / Evidence Cache hit/miss 统计
- [x] 可配置全局 system prompt 前缀，用于在内置工具 prompt 前追加团队/项目级规则
- [x] 通用 MCP Host 配置模板一键复制；更复杂的自动检测仍作为后续扩展

## 0.3 — 通用性与可观测性

- [x] Job 历史 GUI：查看 stage、耗时、LLM 调用/token 统计和失败诊断
- [x] 支持用户配置新 Job 的 TTL / polling interval；过期清理继续沿用启动时自动清理
- 更强的取消：允许主动中断正在执行的 HTTP LLM 请求
- 可选 Tasks `notifications/tasks` / subscriptions UX（当前 polling 已满足可靠性需求）
- 建立各 MCP Host 的 Tasks/timeout 支持矩阵与自动诊断
- Job 数量明显增长后评估 SQLite；当前文件状态 + 跨进程锁保持简单部署
- 多 LLM profile
- [x] `/models` 模型列表刷新与 GUI 快速选择
- [x] 后台 Job 请求耗时 / 输入输出 token / LLM 调用次数统计
- 本地调用历史（默认不保存源码正文）
- [x] 常见 API 鉴权、限流、连接、超时、reasoning 输出耗尽等错误诊断提示
- 客户端适配插件化，降低新增 Coding Agent 的修改范围

## 0.4 — 发布体验

- [x] GUI 模块改为六个一级标签页，持久化原生窗口位置/大小、界面/文字缩放与最后活动标签页
- [x] GitHub Actions：Linux / Windows / macOS 三平台 CI + tag 驱动 Release workflow
- [x] Windows x86_64 NSIS 安装包
- [x] macOS Apple Silicon / Intel `.app` updater bundle + DMG；Developer ID / notarization 配置说明见 `docs/RELEASE.md`
- [x] Linux x86_64 AppImage + Ubuntu/Debian amd64 `.deb`
- [x] GitHub Release `latest.json` + updater 签名 + GUI 自动检查/安装更新
- Social Preview、截图和演示 GIF

## 明确不做（当前）

为了保持工具安全和职责单一，暂不加入：

- 代码修改
- 任意 shell 执行
- Git commit / push
- 自己实现完整 Coding Agent
- 替代任何编码智能体的主模型

LLM2MCP 的核心定位是：**本地上下文读取 + 外部 LLM 深度分析 + MCP 压缩结果回传。**
