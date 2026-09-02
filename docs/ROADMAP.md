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
- [x] English README / 中文 README 双语文档
- [ ] Cursor GUI 实机验证 MCP 发现与工具调用
- [ ] Codex / Claude Code / Grok Build 实机 MCP 验证
- [ ] Pi + MCP extension 实机验证
- [ ] Windows 11 构建与多客户端验证
- [ ] macOS 构建验证

## 0.2 — 上下文选择优化

- `analyze/plan` 两阶段代码发现：先根据 manifest / symbol 信息选择关键文件，再进行深度分析
- 更准确的 token 预算，而不是仅按字符数限制
- 支持 include / exclude glob
- 工具返回文件证据和建议读取范围
- 可配置 system prompt 模板
- 支持更多通用 MCP Host 的自动检测与配置模板导出

## 0.3 — 通用性与可观测性

- Job 历史 GUI：查看 stage、耗时、结果、失败原因和清理状态
- 支持用户配置 Job TTL / polling interval / 清理策略
- 更强的取消：允许主动中断正在执行的 HTTP LLM 请求
- 可选 Tasks `notifications/tasks` / subscriptions UX（当前 polling 已满足可靠性需求）
- 建立各 MCP Host 的 Tasks/timeout 支持矩阵与自动诊断
- Job 数量明显增长后评估 SQLite；当前文件状态 + 跨进程锁保持简单部署
- 多 LLM profile
- `/models` 模型下拉刷新
- 请求耗时 / 输入输出 token 统计
- 本地调用历史（默认不保存源码正文）
- 更完善的错误诊断
- Provider preset：LiteLLM / llama.cpp / vLLM / Ollama
- 客户端适配插件化，降低新增 Coding Agent 的修改范围

## 0.4 — 发布体验

- GitHub Actions：Linux / Windows / macOS 构建
- Windows 安装包
- macOS app bundle / 签名说明
- Linux AppImage 或便携二进制
- 自动检查更新（可选）
- Social Preview、截图和演示 GIF

## 明确不做（当前）

为了保持工具安全和职责单一，暂不加入：

- 代码修改
- 任意 shell 执行
- Git commit / push
- 自己实现完整 Coding Agent
- 替代任何编码智能体的主模型

LLM2MCP 的核心定位是：**本地上下文读取 + 外部 LLM 深度分析 + MCP 压缩结果回传。**
