# LLM2MCP 架构

## 数据流

```text
AI 编码智能体主模型
Cursor / Codex / Claude Code / Grok Build / Pi / 其他 MCP Client
   │
   │ stdio MCP，仅传 task / paths
   ▼
LLM2MCP（开发机本地）
   ├─ 解析当前 workspace
   ├─ 校验 workspace 边界
   ├─ 遵循 .gitignore 扫描
   ├─ 排除常见 secrets
   ├─ 本地读取源码 / git diff
   ├─ document_repo：仓库分块 → Repository Map → 文档合成
   ├─ update_docs：Git diff + 现有文档 → 增量文档更新
   │
   │ OpenAI-compatible /v1/chat/completions
   ▼
用户配置的 LLM API
   │
   ▼
Qwen / llama.cpp / LiteLLM / vLLM / Ollama / 其他兼容服务
   │
   ▼
压缩后的分析、计划、review 或项目文档
   │
   ▼
AI 编码智能体主模型
```

## 进程模型

同一个二进制有四种运行模式：

- `llm2mcp`：启动 GUI，用于配置 API、语言、工具 reasoning/execution 和编码智能体集成。
- `llm2mcp install`：把当前版本刷新到稳定的用户级可执行文件路径并打印该路径。
- `llm2mcp mcp [--workspace <path>]`：编码智能体自动拉起的 stdio MCP 子进程。
- `llm2mcp job-worker --job-id <id>`：内部隐藏模式，负责执行已经持久化的后台 Job。它由 MCP 进程自动启动，不需要用户手工运行。

`--workspace` 可选：

- Cursor 使用 `${workspaceFolder}` 显式传入。
- Codex / Claude Code / Grok Build / Pi 等通用 stdio 客户端默认继承当前工作目录，LLM2MCP 直接把该目录作为 workspace。

GUI 不需要常驻，也不开放任何本地 HTTP 端口。首次启动默认使用 `1280×900` 窗口；主界面不再使用并排等高卡片，而是把 `LLM API / 上下文限制 / 工具独立思考强度 / System Prompt / Job 历史 / AI 编码智能体` 分成六个一级标签页，每次只显示当前模块。eframe persistence 保存原生窗口位置/大小，LLM2MCP 另外持久化 egui zoom factor 和最后打开的标签页，因此用户调整后的界面/文字缩放和工作位置会在下次启动恢复。GUI 还提供 OpenAI-compatible `/models` 刷新、可选全局 System Prompt 前缀，以及通用 MCP JSON 配置模板复制。

长任务的进程生命周期与 MCP 请求解耦：

```text
MCP Host
   │ tools/call
   ▼
LLM2MCP MCP process
   │ 1. durable job.json
   │ 2. spawn job-worker
   │ 3. immediately return job/task handle
   ▼
MCP Host 可结束本次调用甚至重启 MCP process

独立 job-worker
   │
   ├─ workspace scan / git diff
   ├─ LLM calls
   ├─ stage/progress 持久化
   └─ final result 持久化

后续任意新的 MCP process
   │
   └─ job_status / job_result 或 tasks/get
```

## 客户端适配层

客户端差异只存在于“如何注册 MCP”，MCP server 本身保持统一。执行“安装 / 更新”时会先刷新稳定的用户级 LLM2MCP 可执行文件，再让客户端配置指向该稳定路径，而不是开发目录里的临时构建产物：

- Cursor：直接合并 `~/.cursor/mcp.json`。
- Claude Code：调用官方 `claude mcp add/remove`。
- Codex：调用官方 `codex mcp add/remove`。
- Grok Build：调用官方 `grok mcp add/remove`。
- Pi：安装 MCP extension 后维护 `~/.pi/agent/mcp.json`。
- 其他客户端：使用标准 `llm2mcp mcp` stdio 启动命令。

对于 LLM2MCP 自己修改的 JSON/TOML 客户端配置，旧文件先复制为 `*.llm2mcp.bak`，新内容通过同目录临时文件写入并原子替换，降低配置中途损坏的风险。这种设计既避免把项目绑定到某一个 IDE/Agent，也避免正常安装后后台 worker 因 `target/debug/...` 被重新构建替换而失去可执行路径。

安装器还会把 Host timeout 当作第二层保险：

- Codex：安装后尝试设置 `tool_timeout_sec = 1800`。
- Grok Build：安装后尝试设置 `tool_timeout_sec = 6000`。
- Claude Code：用户级配置中尝试设置约 30 分钟的 MCP request/tool timeout。
- Cursor / Pi：不依赖可调 timeout，主要通过 Async Job 规避 Host 的硬超时。

这些 timeout 不是长任务架构的前提；即使 Host timeout 较短，Async Job 仍应在原始 `tools/call` 超时之前返回 handle。

## 多语言

GUI 文案通过独立 `i18n` 层提供，目前支持：

- English
- 简体中文

Linux / Windows / macOS 启动 GUI 时会尝试加载系统 CJK 字体作为 egui fallback，不把第三方字体打包进发行物。

## 安全模型

0.1 MCP 只读：

- 不执行任意 shell；`review_diff` 以及 `debug_issue(recent_changes=true)` 只通过固定参数调用受约束的 `git status` / `git diff` 读取变更证据。
- 请求路径 canonicalize 后必须位于当前 workspace 内。
- symlink 解析后若越界会被拒绝。
- 默认遵循 `.gitignore` / Git exclude。
- 默认排除 `.env*`、私钥、credentials、secrets 等常见敏感文件。

注意：用户主动选择用于分析的源码会发送到其配置的 LLM API。

## 上下文发现、过滤与 token 预算

`analyze` / `debug_issue` / `plan` 对目录、多路径、空 `paths` 或明显较大的单文件不再直接顺序读取大量正文，而是采用两阶段发现：

1. 本地按 workspace sandbox、`.gitignore`、secret 规则和可选 `include` / `exclude` glob 枚举候选文本文件。
2. 对候选文件建立带精确 `start_line/end_line` 的轻量 symbol index，覆盖函数、结构体、类、接口、类型、Markdown 标题等声明；Discovery Index 有独立 token 上限。
3. 先用本地 lexical/symbol scoring 对 task 与 path/symbol label 做相关性排序。匹配足够强时直接选中最多 12 个 symbol/file，完全跳过额外 LLM discovery。
4. 只有本地评分不够确定时，才使用 Low reasoning 的短 LLM 调用对 Discovery Index rerank；返回的路径和行范围必须逐项精确匹配本地候选，模型虚构内容会被丢弃。
5. 深度阶段优先只读取被选中的 symbol range，并在源码中附真实行号、少量前后文以及最多约 30 行文件头/import context；只有没有合适 symbol 时才回退到整文件读取。
6. 最终 `analyze` / `debug_issue` / `plan` 请求只接收这批收敛后的上下文。`debug_issue` 还会把用户提供的日志/错误和可选当前 Git diff 作为额外运行时证据；源码上下文会预留一部分 token budget 给这些证据，避免总输入无界增长。

Discovery 的文件/symbol 解析结果不会每个 Job 从零开始。LLM2MCP 在本机应用数据目录维护版本化的 **Repository Symbol Index**：按 canonical workspace 分桶，记录每个候选文件的 `size + modified_ns`、内容 SHA-256、总行数和 symbol ranges。文件 stamp 未变化时直接复用已有 hash/ranges，避免重新读取源码正文；只有新增或变化文件才重新读取、哈希和解析，再通过跨进程文件锁合并进索引。

深度读取还有独立的 **Evidence Cache**。缓存 key 由版本、workspace、相对路径、文件内容 SHA-256 和精确证据段（prelude 或 symbol/context 行范围）组成。相同代码在连续 `analyze → debug_issue → plan` 或重复任务中可以直接复用格式化好的带行号证据；源码变化后内容哈希改变，旧证据自然失效。真正使用缓存前仍会用当前 symbol index 验证选中 range，避免 source 在 Discovery 与深度读取之间变化时继续使用旧范围。缓存目录/文件在 Unix 下分别收紧为 `0700/0600`，只保存在本机，不作为额外源码历史回传 MCP Host。

较小的单个显式文件仍直接深度读取，避免为简单任务增加发现调用。`document_repo` 不做 symbol 选择，但同样支持 `include` / `exclude` glob 来约束 Repository Map 的扫描范围。

源码、单文件、Discovery Index 和 Git diff 的主要上下文限制现在使用 token 估算，而不是仅比较字符数。估算器对 ASCII 标识符按近似子词长度计数，对 CJK 和标点按更细粒度计数；它不是 Provider tokenizer 的逐 token 精确复刻，但对中英文混合代码比固定字符上限更接近真实模型上下文占用。旧 `max_source_chars` / `max_file_chars` 配置字段仅用于旧配置迁移，新 GUI 使用 source/file/discovery token budget。

`analyze`、`debug_issue`、`plan`、`review_diff` 还将“副模型可以生成多少”和“主模型最终收到多少”拆成两个预算。副模型默认输出预算分别约为 4K / 6K / 6K / 4K；LLM2MCP 在 Rust 本地解析结构化 JSON，不再调用第二次 LLM 总结，然后按默认 800 / 1400 / 1200 / 1000 token 的 Primary return budget 做硬上限裁剪。`debug_issue` 的结构额外包含 diagnosis、confidence、root cause、execution path、intermittency、alternative hypotheses、verification 和 likely fix area。`read_next` 放在返回最前面，并且 `analyze/debug_issue/plan` 的 `path + line range` 必须能映射回本地选中的 symbol range；因此主编码智能体可以直接读取几十到几百行，而不是再次打开完整文件。文档生成工具属于最终内容产物，不做这一层压缩。

## Reasoning 适配

LLM2MCP 将“工具该思考多深”和“Provider 如何接收思考参数”分开：

- 每个工具独立设置 `Off / Low / Medium / XHigh`。
- 全局选择参数传输方式：`None / OpenAI / Qwen / thinking`。

这样 `plan` / `update_docs` 可以默认比 `analyze` 使用更高 reasoning；`document_repo` 则默认使用 Medium，并在最终 Reduce 无 final content 时自动降低 reasoning 重试，而不要求所有模型都实现同一种参数格式。

## 执行策略与 Durable Job Manager

每个主要工具同时配置 reasoning、output budget 和 execution mode：

- `Sync`：在原 `tools/call` 内执行。
- `Auto`：根据请求规模选择 Sync/Async；当前目录、多路径或明显较大的文件会转 Async。
- `Async`：持久化创建 Job 并立即返回。

默认策略：

- `analyze` → Auto
- `debug_issue` → Auto（大日志、`recent_changes=true`、宽目录/多路径更倾向 Async）
- `plan` → Auto
- `review_diff` → Sync
- `document_repo` → Async
- `update_docs` → Async

Job 状态包含 tool、workspace、arguments、stage、progress、total、result/error、创建/更新时间和取消标记。每次有 usage 信息的 LLM 响应还会累计 `llm_calls`、`prompt_tokens`、`completion_tokens`、最后一次 LLM diagnostics 和总耗时；`analyze/debug_issue/plan` 另外累计 `symbol_index_hits/misses` 与 `evidence_cache_hits/misses`，用于确认持久化缓存是否真正减少本地扫描/证据抽取。失败时根据常见 401/403、429、连接失败、timeout、reasoning 输出预算耗尽、过滤后无源码等情况附带诊断建议。默认保留 7 天；GUI 可调整新建 Job 的 TTL 和 Host 建议轮询间隔，已创建 Job 保留其创建时的设置。

GUI 会读取最近的持久化 Job JSON，显示状态、stage、耗时、输入/输出 token、LLM 调用次数、Symbol Index / Evidence Cache 命中率、失败原因和诊断建议；Job 历史本身仍不额外保存源码正文。

持久化实现遵循两个原则：

1. Unix 使用原子 rename 替换完整 Job JSON，查询方不会看到半写状态；Windows 使用兼容的替换流程。
2. Job 更新使用跨进程文件锁，避免 worker 的进度更新与 `job_cancel` 同时写入时相互覆盖。

取消是协作式：worker 在阶段边界和不同 LLM 调用之间检查取消状态；已经发出的阻塞 HTTP LLM 请求目前不会被强制中断。

## MCP Tasks + Legacy Job Fallback

LLM2MCP 同时支持两条长任务路径：

**现代 Host（MCP `2026-07-28`）**

- `server/discover` 广告 `io.modelcontextprotocol/tasks`。
- 客户端需要在当前请求 `_meta` 中显式声明 Tasks extension。
- 长工具返回 `resultType: "task"`。
- `tasks/get` 返回持久化状态，完成后 `result` 内嵌原始 CallToolResult。
- `tasks/cancel` 请求协作式取消。
- 未声明 Tasks capability 的客户端调用 `tasks/*` 会被拒绝。

**Legacy / Tasks-unaware Host**

- 原工具调用快速返回普通 CallToolResult + `structuredContent.job_id`。
- `job_status(job_id)` 查询进度。
- `job_result(job_id)` 获取最终原始工具结果。
- `job_cancel(job_id)` 请求取消。

现代响应按照最终 2026-07-28 规范在结果 `_meta["io.modelcontextprotocol/serverInfo"]` 中携带 server identity；legacy `initialize` 路径继续保持 2025-era 行为。

我们不依赖 `notifications/progress` 来防止 Host timeout。进度通过持久化 Job polling / `tasks/get` 提供；这样硬 timeout 不会决定后台 LLM 的生命周期。

## Repository Documentation Map→Reduce

`document_repo` 不会简单读取仓库前 N 个字符。它采用有上限的多轮处理：

1. 本地遵循 `.gitignore`、workspace sandbox、secret 规则以及可选 `include` / `exclude` glob 枚举文本文件。
2. 将源码按 token 估算的单次上下文预算分块，当前最多处理 12 个 chunk。
3. 每个 chunk 由辅助 LLM 使用 Low reasoning 生成约 800–1200 tokens 的 Repository Map 摘要，记录模块职责、入口、符号、数据流、API、配置、部署信息和证据路径；避免把昂贵的深度推理浪费在事实抽取阶段。
4. Map 默认并发数为 2（GUI 可配置 1–8）。每个成功 summary 会立即写入内容寻址的 checkpoint/cache；cache key 包含 Map prompt 版本、API/model 配置、输出预算、文件路径和 chunk 内容，因此代码未变化的 chunk 可跨 Job、跨文档类型复用。
5. Reduce 阶段只读取 manifest + Map summaries。`document_repo` 默认 Medium reasoning；若响应只有 `reasoning_content`、最终 `message.content` 为空，则自动按 `XHigh → Medium → Low` 或 `Medium → Low` 降级，只重试 Reduce，不重复 Map。
6. 如果所有 Reduce 尝试仍失败，Job 保持 `progress=<map_chunks>/<map_chunks+1>` 并标记 `state=failed / stage=reduce_failed`；错误记录包含 `finish_reason`、prompt/completion tokens 和 reasoning/content 长度，避免把失败误报成 `completed`。
7. 输出使用 `===== DOCUMENT: path =====` 边界返回给主编码智能体，LLM2MCP 自己不写文件。

`update_docs` 则走增量路径：

- 默认比较 `HEAD -> WORKTREE`，也支持任意两个 Git ref。
- 自动发现 `README.md`、`README.zh-CN.md` 和 `docs/**/*.md`，也可以显式指定文档列表。
- 将代码 diff 与现有文档一起交给辅助 LLM，只返回真正需要修改文档的完整替换 Markdown。

这两个工具都保持只读，因此“理解/生成”与“最终写文件”仍由 MCP Host/主编码智能体分离。

## 0.1 MCP 协议范围

当前内置 stdio JSON-RPC 实现同时服务两代 MCP：

Legacy / handshake era：

- `initialize`
- `ping`
- `tools/list`
- `tools/call`

Modern `2026-07-28`：

- `server/discover`
- 每请求 `_meta` protocol/capabilities
- `tools/list`
- `tools/call`
- Tasks extension：`tasks/get / tasks/update / tasks/cancel`

另外所有 Host 都可使用 LLM2MCP 自己的 `job_status / job_result / job_cancel` fallback tools。

stdout 只输出 MCP JSON-RPC；worker 日志和 MCP 运行日志都不会污染 stdout。随着协议覆盖面继续扩大，后续仍可评估迁移到官方 Rust SDK `rmcp`，但 Job Manager 与业务工具保持独立，不依赖具体 SDK。

## Runtime hardening and model routing

`control.rs` propagates cancellation through request threads and Map workers. `process.rs` bounds client command waits, while `scheduler.rs` holds shared file-lock leases for job and HTTP slots. The stdio loop dispatches business requests without holding stdout locks during work; each complete reply is written atomically. Durable jobs retain credential-free routing snapshots and heartbeat leases, and readers share the state lock with writers (including Windows replacement).

`syntax.rs` supplies Tree-sitter symbols and lexical import hints; `search.rs` supplies Chinese aliases and path/label scores. Discovery ranks files before emitting its token-bounded index. Evidence is masked before caching and again at the outbound HTTP boundary. `cache.rs` applies on-access age/size pruning across source/index and map caches.

Tool routing selects a named profile first. Discovery/Map optionally override that profile, while Reduce uses the tool profile. `doctor.rs` separates inference/stdio probes from host registration. Both sync and async requests record durable metrics, and the GUI consumes background events for operations and history refresh. Detailed guarantees and limits are documented in [RUNTIME.md](RUNTIME.md).
