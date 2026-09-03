# LLM2MCP

[English](README.md) | [简体中文](README.zh-CN.md)

LLM2MCP 是一个跨平台、单二进制的本地 MCP Bridge：把任意 OpenAI-compatible LLM API 变成 AI 编码智能体可调用的“本地代码智能副模型”。

它的核心用途是 Context Offload：当大目录、大文件或大 diff 不值得全部塞进主编码智能体上下文时，LLM2MCP 在开发机本地读取选中的代码，把上下文直接发送给你配置的 LLM，只把压缩后的分析、实现计划或代码复审结果返回主编码智能体。

LLM2MCP 使用本地 stdio MCP，不需要额外开放本地 HTTP 端口，也不会覆盖 Cursor、Codex、Claude Code、Grok Build 等工具原有的模型配置。

## 主要能力

- Windows / Linux / macOS 单二进制。
- English / 简体中文 GUI，采用六个模块标签页（`LLM API / 上下文限制 / 工具独立思考强度 / System Prompt / Job 历史 / AI 编码智能体`）；原生窗口位置/大小、界面/文字缩放和上次打开的标签页都会在下次启动时恢复。
- GUI 配置 OpenAI-compatible API 地址、API Key、模型、reasoning 参数协议和可选全局 System Prompt 前缀，并支持刷新 OpenAI-compatible `/models` 列表。
- 每个 MCP 工具独立设置思考强度、执行方式（`Sync / Auto / Async`）和输出预算。
- 支持多个 AI 编码智能体，并保留通用 stdio MCP 接入方式。
- MCP 只读访问当前工作区，并限制路径不能越界。
- 遵循 `.gitignore`，默认排除常见密钥和凭据文件。
- 将大上下文分析下沉到自托管/自定义 LLM，让主编码智能体只读取压缩后的结果。
- 通过有上限的 Map→Reduce 扫描生成项目文档，并根据 Git 变更增量更新现有文档。
- 持久化后台 Job 让较慢的本地模型运行数分钟，也不会长期占住单次 MCP `tools/call`。
- Host 支持时使用 MCP `2026-07-28` Tasks Extension；旧 Host 则通过通用 `job_status / job_result / job_cancel` 工具兼容。

## 支持的 AI 编码智能体

点击 **“安装 / 更新”** 时，LLM2MCP 会先把当前程序刷新到稳定的用户级可执行文件路径，再让目标编码智能体注册这个稳定路径。这样 MCP 配置不会继续指向 `target/debug/...` 等可能在重新构建或升级后消失的开发路径。对于 LLM2MCP 直接修改的 JSON/TOML 客户端配置，写入前会保留 `*.llm2mcp.bak` 备份，并通过临时文件 + 原子替换更新配置。

### Cursor

LLM2MCP 会合并用户级 `~/.cursor/mcp.json`，Cursor 通过 `${workspaceFolder}` 把当前工作区传给 LLM2MCP。

### Claude Code

通过 `claude mcp` 命令安装用户级 stdio MCP。

### Codex

通过 `codex mcp` 命令安装 stdio MCP。

### Grok Build

通过 `grok mcp` 命令安装 stdio MCP。

### Pi

Pi 当前需要 MCP 扩展。LLM2MCP 可以尝试安装 `pi-mcp-extension`，然后写入用户级 MCP 配置。

### 其他 MCP 客户端

任何能够启动本地 stdio MCP 的编码智能体都可以直接使用：

```bash
llm2mcp mcp
```

如果不传 `--workspace`，LLM2MCP 会把 MCP 客户端启动进程时的当前工作目录作为 workspace 根目录。

也可以显式指定：

```bash
llm2mcp mcp --workspace /path/to/project
```

对于没有专用安装器的 MCP Host，GUI 可以一键复制指向稳定用户级可执行文件的 `mcpServers.llm2mcp` 通用 JSON 配置模板。

## MCP Tools 使用说明

LLM2MCP 提供六个主要只读工具，以及三个通用后台 Job 工具：

| Tool | 适合场景 | 必填参数 | 可选参数 | 默认思考强度 | 默认执行方式 |
| --- | --- | --- | --- | --- | --- |
| `analyze` | 根因分析、架构分析、大文件/目录分析 | `task` | `paths`, `include`, `exclude` | Medium | Auto |
| `debug_issue` | 根据故障现象、日志、源码证据和可选近期 Git 变更定位具体问题 | `issue` | `paths`, `logs`, `expected`, `actual`, `recent_changes`, `include`, `exclude` | XHigh | Auto |
| `plan` | 独立制定实现计划、提供第二意见 | `task` | `paths`, `include`, `exclude` | XHigh | Auto |
| `review_diff` | 不让主模型先读取完整 diff 的代码复审 | 无 | `task`, `base_ref` | Medium | Sync |
| `document_repo` | 分块扫描代码仓库并通过 Map→Reduce 生成有依据的项目文档 | 无 | `document_type`, `paths`, `include`, `exclude`, `audience`, `language` | Medium | Async |
| `update_docs` | 根据 Git 变更增量更新现有 README/docs，避免重新扫描整个仓库 | 无 | `base_ref`, `target_ref`, `docs`, `language` | XHigh | Async |

每个主要工具都可以在 GUI 中单独配置 `Off / Low / Medium / XHigh` 思考强度、`Sync / Auto / Async` 执行方式，以及副模型最大输出 token 数。`analyze`、`debug_issue`、`plan`、`review_diff` 另外提供独立的 **回传主模型** budget：副模型内部可以使用更大的输出预算充分分析，但 LLM2MCP 会在本地把结构化结果压缩后再送入主编码智能体上下文。默认分别为 `analyze=800`、`debug_issue=1400`、`plan=1200`、`review_diff=1000` tokens；文档工具则返回完整产物。输入上下文也按 token 估算控制：GUI 分别提供整体源码、单文件和轻量发现索引的 token budget。该估算器保持 Provider 无关，不假装能够精确复刻所有 OpenAI-compatible 模型的 tokenizer，但比固定字符数更适合中英文混合代码。

三个 Job 工具始终是快速、只读调用：

- `job_status(job_id)` — 获取阶段、进度和状态。
- `job_result(job_id)` — 获取原始最终结果；尚未完成时返回当前状态。
- `job_cancel(job_id)` — 请求协作式取消后台任务。

### `analyze`

当主编码智能体原本需要读取大量源码、日志、配置文件或多个相关模块时，优先使用 `analyze`。

参数：

- `task` — 必填。说明要分析什么，以及需要回答哪些问题。
- `paths` — 可选。workspace 相对路径组成的文件/目录数组。目录、多路径或空数组会先进行轻量仓库发现，再读取真正相关的源码；单个显式文件会直接读取，避免额外发现调用。
- `include` — 可选 glob 数组，例如 `src/**/*.rs`，只有匹配的源码候选会进入发现/读取。
- `exclude` — 可选 glob 数组，例如 `tests/**` 或 `*.generated.ts`，排除规则优先于 include。

发现阶段会在本机生成受 token 上限约束的“文件路径 + symbol + 精确行范围”索引，并优先使用本地 lexical/symbol scoring。匹配足够明确时直接选中相关 symbol，完全跳过额外的发现 LLM；只有语义模糊时才使用一次 Low reasoning 的短请求对轻量索引 rerank。第二阶段优先只读取被选中的 symbol 行范围，并带少量文件头/import 上下文和真实行号，而不是整个文件。模型虚构的路径或行范围因为无法匹配本地候选索引会被丢弃。

文件/symbol 索引现在会跨 Job 持久化。LLM2MCP 在本机应用数据目录保存按 workspace 隔离的 **Repository Symbol Index**，先用文件大小 + 纳秒级 mtime 快速判断是否未变化；未变化文件直接复用已有 SHA-256 和 symbol ranges，不再重新读取源码正文和解析声明，只有新增/变化文件才重新读取、内容哈希和解析。被实际选中的源码证据另存为 **Evidence Cache**，key 包含文件内容 SHA-256 + 精确行段，因此连续 `analyze → plan` 或重复分析可以直接复用相同 symbol/prelude 片段；源码一旦变化会自然生成新 key，同时选中的旧行范围还会再次对当前 symbol index 校验，不会静默复用过时证据。缓存仅保存在 LLM2MCP 本机数据目录，不作为历史正文回传主模型；Job status/历史会显示 Symbol Index 和 Evidence Cache 的 hit/miss 数量，便于确认优化是否生效。

推荐给主编码智能体的提示词：

```text
不要先读取整个 src 目录。
先调用 LLM2MCP analyze 分析 src/auth 和 src/session。
找出间歇性登录失败的根因，给出相关文件/符号作为证据，
最后只告诉我接下来真正需要打开哪些文件。
```

典型 tool 参数：

```json
{
  "task": "找出间歇性登录失败的根因，并指出真正需要修改的文件",
  "paths": ["src/auth", "src/session"]
}
```

副模型返回结构化分析后，LLM2MCP 不再额外调用一次 LLM 做总结，而是在本地按 **回传主模型** budget 裁剪、去掉低优先级内容。结果最前面优先放经过本地校验的 `READ_NEXT`，例如 `src/jobs.rs:400-445 — fn spawn_worker(...)`，让主编码智能体只读取真正相关的几十行，而不是整个文件。

### `debug_issue`

当程序已经出现明确故障，目标是回答“**为什么坏了**”时使用 `debug_issue`。它复用 `analyze` 已有的两阶段文件/symbol 发现、精确源码范围和 Evidence Cache，但返回结构专门面向 Debug：诊断结论、置信度、最可能根因、执行路径、间歇性原因、备选假设、验证方法、可能修复区域和经过本地校验的 `READ_NEXT`。

参数：

- `issue` — 必填。描述故障现象。
- `paths` — 可选。优先检查的 workspace 相对文件/目录。
- `logs` — 可选。可以传 stack trace、运行时/编译错误、HTTP 错误、浏览器 console、服务日志等观测证据。
- `expected` / `actual` — 可选。预期行为和实际行为。
- `recent_changes` — 可选布尔值；为 `true` 时会把当前 tracked Git diff（相对 `HEAD`）作为额外诊断证据。
- `include` / `exclude` — 可选源码 glob。

在 `Auto` 模式下，大日志、近期 Git diff 或宽范围目录会更倾向转为 Async。日志和 diff 会与源码共享配置的 source-token 预算，不会绕过上下文上限。`debug_issue` 仍然保持只读：不会执行程序、运行测试、修改文件或重启服务，只会把建议验证步骤返回给主编码智能体执行。

典型参数：

```json
{
  "issue": "API 后台 worker 偶尔启动失败并报 No such file or directory",
  "paths": ["src/jobs.rs", "src/install.rs"],
  "logs": "failed to start job worker: No such file or directory (os error 2)",
  "expected": "MCP 请求返回后 durable worker 可以正常启动",
  "actual": "Job 立即标记为 failed",
  "recent_changes": true
}
```

### `plan`

在较大功能、重构、迁移或复杂修复开始前使用 `plan`，让本地/自定义 LLM 独立制定一份方案，作为主编码智能体的第二意见。

参数：

- `task` — 必填。说明要实现的功能、修复、迁移或重构目标。
- `paths` — 可选。需要规划时参考的 workspace 相对文件/目录。目录、多路径或空数组与 `analyze` 一样走“本地 scoring → 必要时 LLM rerank → symbol/line-range 深度读取”的路径。
- `include` — 可选 include glob。
- `exclude` — 可选 exclude glob，排除规则优先。

推荐提示词：

```text
实现之前先调用 LLM2MCP plan。
使用 src/cache、src/image 和 Cargo.toml 作为上下文。
让辅助 LLM 给出最小实现方案、涉及的模块、风险和测试方案。
把它作为第二意见综合判断，不要直接照搬。
```

典型 tool 参数：

```json
{
  "task": "设计一个持久化缩略图缓存方案，尽量少改代码，并避免影响现有图片加载逻辑",
  "paths": ["src/cache", "src/image", "Cargo.toml"]
}
```

返回内容重点包括：目标和约束、推荐实现、需要修改的文件/模块、执行顺序、风险、兼容性和验证方式。

### `review_diff`

代码修改完成后使用 `review_diff`。LLM2MCP 会在本机读取 Git diff，并直接发送给配置的 LLM，不需要主编码智能体先把完整 diff 吃进上下文。

参数：

- `task` — 可选。默认检查当前修改的正确性和回归问题。
- `base_ref` — 可选。`git diff` 使用的 Git 基准版本，默认 `HEAD`。

推荐提示词：

```text
结束任务前调用 LLM2MCP review_diff。
检查当前 tracked changes 是否存在正确性问题、回归、并发问题、API 兼容问题和缺失测试。
只把重要发现带回当前对话，不要复述整个 diff。
```

典型 tool 参数：

```json
{
  "task": "检查当前修改中的正确性问题、竞态条件、回归和缺失测试",
  "base_ref": "HEAD"
}
```

如果当前没有 tracked Git diff，工具会直接返回没有可复审的 tracked diff。

### `document_repo`

需要让辅助 LLM 理解整个代码仓库并生成开发者文档时使用 `document_repo`。与 `analyze` 不同，`document_repo` 的 `paths` 为空时会主动递归扫描仓库。LLM2MCP 会在本机把可读取源码分成有上限的多个 chunk，逐块生成 Repository Map 摘要，再根据这些摘要合成最终文档，而不是简单截断仓库前一部分代码。

参数：

- `document_type` — 可选。支持 `overview`、`architecture`、`modules`、`api`、`developer`、`deployment`、`full`，默认 `overview`。
- `paths` — 可选。workspace 相对文件/目录。为空表示扫描整个仓库。
- `include` — 可选。限制仓库扫描范围的 include glob。
- `exclude` — 可选。扫描时应用的 exclude glob，优先于 include。
- `audience` — 可选。目标读者，例如 `developer`、`new_contributor`、`operator`，默认 `developer`。
- `language` — 可选。`auto`、`english`、`simplified_chinese`，默认 `auto`。

推荐提示词：

```text
不要由主模型自己先读取整个仓库。
调用 LLM2MCP document_repo 扫描项目，为开发者生成架构文档；
在有代码依据时加入 Mermaid 图，并引用具体文件、模块和符号。
只返回生成的 Markdown，不要自动写文件。
```

典型 tool 参数：

```json
{
  "document_type": "architecture",
  "audience": "developer",
  "language": "simplified_chinese"
}
```

`full` 模式会要求生成一组建议文档：

```text
docs/PROJECT_OVERVIEW.md
docs/ARCHITECTURE.md
docs/MODULES.md
docs/API.md
docs/DEVELOPMENT.md
docs/DEPLOYMENT.md
```

LLM2MCP 仍然保持只读。生成内容使用 `===== DOCUMENT: path =====` 分隔，交给主编码智能体审核后再决定是否真正写入仓库。

`document_repo`，尤其是大仓库的 `full` 模式，本身属于长耗时任务，可能需要连续多次 LLM 调用。它默认使用 **Async**：原始 MCP 调用会很快返回持久化 job/task handle，而不是挂几分钟等待模型。Map 阶段固定使用 **Low** reasoning，并将单 chunk 摘要压缩到约 **800–1200 tokens**；Repository Map 默认并发数为 **2**，可在 GUI 中调整。每个成功 chunk 会立即写入内容寻址的 checkpoint/cache，因此后续 Reduce 失败后重新调用同一仓库时，只会重算发生变化或未命中的 chunk，不会再从头扫描全部仓库。

最终 Reduce 默认使用 **Medium** reasoning。如果用户配置的 reasoning 返回空 `message.content`（例如 `finish_reason=length`，只有 `reasoning_content`），LLM2MCP 会自动复用同一批缓存 Map summaries，并降低 reasoning 重试：`XHigh → Medium → Low`，或 `Medium → Low`。若所有 Reduce 都失败，Job 会正确标记为 `failed / reduce_failed`，不会再误报 `completed`；错误信息会包含可用的 `finish_reason`、prompt/completion token 数，以及 reasoning/content 长度，便于定位是否为输出预算被思考耗尽。

### `update_docs`

代码发生变化、怀疑 README 或 docs 已经过时时使用 `update_docs`。LLM2MCP 会在本机读取 Git diff，默认自动发现 README 和 `docs/**/*.md`，只让辅助 LLM 返回真正受影响文档的完整替换 Markdown。这个工具要求当前 workspace 是 Git 仓库，并且指定的 commit/tag/ref 可以访问。

参数：

- `base_ref` — 可选，默认 `HEAD`。
- `target_ref` — 可选，默认 `WORKTREE`，表示当前 staged/unstaged tracked changes；也可以指定 `main`、`v0.2.0`、commit/tag 等 Git ref。
- `docs` — 可选。指定需要检查的 workspace 相对 Markdown 文件；为空时自动发现 `README.md`、`README.zh-CN.md` 和 `docs/**/*.md`。
- `language` — 可选。`auto`、`english`、`simplified_chinese`。

推荐提示词：

```text
调用 LLM2MCP update_docs 检查 HEAD -> WORKTREE。
判断当前代码修改是否让 README 或 docs 过时。
只返回真正需要修改的文档，先让我审核，不要自动写文件。
```

典型 tool 参数：

```json
{
  "base_ref": "HEAD",
  "target_ref": "WORKTREE",
  "language": "auto"
}
```

`update_docs` 同样默认使用 **Async**，因此较慢的文档合成不会长期占住 Host 的原始 MCP 调用。

发布版本之间也可以直接比较：

```json
{
  "base_ref": "v0.1.0",
  "target_ref": "v0.2.0",
  "docs": ["README.md", "docs/ARCHITECTURE.md"]
}
```

## 长耗时任务与慢速本地模型

本地/自托管模型在仓库级任务上可能需要几分钟。LLM2MCP 因此不会让慢任务长期占住同一个 `tools/call`。

每个主要工具可以独立选择执行方式：

- **Sync / 同步** — 原始 MCP 调用一直等待 LLM 返回，适合规模小、耗时稳定的请求。
- **Auto / 自动** — 小请求同步执行；目录、多文件或明显较大的请求自动转成后台 Job。
- **Async / 异步** — 先持久化创建后台 Job，再立即返回。`document_repo` 和 `update_docs` 默认使用此模式。

对于旧版或不支持 Tasks Extension 的 MCP Host：

```text
tools/call document_repo
    ↓ 很快返回
job_id
    ↓
job_status(job_id)
    ↓ 按需重复查询
job_result(job_id)
```

LLM2MCP 会先把 Job 写入本机应用数据目录，再返回 job handle。真正的工作由独立 `llm2mcp job-worker` 子进程执行，因此原来的 MCP stdio 进程即使退出、超时或重新启动，Job 状态和最终结果仍然可以恢复。

编码智能体集成现在统一指向稳定的用户级可执行文件，而不是临时开发构建路径，因此正常安装后的后台 worker 不再依赖 `target/debug/...`，可以直接规避这类重建后路径失效问题。Linux 仍保留 `/proc/self/exe` 作为开发期或特殊原地更新场景的后备路径：如果已经运行的 MCP 进程发现原始可执行文件路径被删除，仍可通过当前进程镜像启动同版本 worker。LLM2MCP 本身升级后仍建议重启/刷新一次 MCP 连接，让 Host 加载新版 server。

如果 MCP Host 使用 `2026-07-28` 协议并显式声明支持 `io.modelcontextprotocol/tasks`，LLM2MCP 会优先返回标准 `resultType: "task"` handle；Host 可以使用 `tasks/get` 轮询，完成后会直接得到原工具的最终结果。对于尚未支持 Tasks 的 Host，`job_status / job_result / job_cancel` 继续作为通用兼容路径。

Job 的进度会持久化，例如：

```text
scanning repository
mapping repository 3/8
synthesizing documentation
completed
```

`job_cancel` / `tasks/cancel` 是协作式取消：LLM2MCP 会在阶段切换和不同 LLM 调用之间检查取消状态；如果某个 HTTP LLM 请求已经在执行，它可能会先完成该请求，再响应取消。

Job 默认保留 7 天，LLM2MCP 启动时会顺带清理过期的 Job 状态和日志文件。GUI 可以调整新建 Job 使用的保留 TTL 和 Host 轮询间隔。持久化 Job 还会累计总耗时、LLM 调用次数、Provider 返回的 prompt/completion token 用量、最后一次 LLM diagnostics，并针对常见鉴权失败、限流、连接失败、timeout、reasoning 输出预算耗尽和过滤后无源码等问题给出诊断建议。GUI 增加了可刷新的最近 Job 历史面板，但不会为了历史记录额外保存一份源码正文。

作为第二层保险，安装到支持配置的编码智能体时，LLM2MCP 会尝试设置更宽松的 MCP tool timeout。但 **Async Job 才是解决 Host 硬超时的主要机制**，尤其适用于无法可靠调整 timeout 的 Host。

## 推荐的 Context Offload 工作流

LLM2MCP 最大的价值来自**不要重复读取同一批上下文**。

推荐：

```text
主编码智能体
    ↓ 只传 task + paths
LLM2MCP
    ↓ 在本机读取大量源码
配置的 LLM API
    ↓ 返回压缩后的 2K～4K 风格报告
主编码智能体
    ↓ 只打开真正相关的几个文件
实施修改
    ↓
LLM2MCP review_diff
```

不推荐：

```text
主编码智能体已经先读取 50K token 源码
    ↓
然后又让 LLM2MCP 分析同样的 50K token
```

这种方式会重复消耗上下文，明显降低节省 token 的意义。

可以给主编码智能体固定一条规则：

```text
遇到大上下文探索任务时优先使用 LLM2MCP。
调用前不要读取整个目录，只把 workspace 相对路径传给工具；
拿到报告后，再只打开报告指出的关键文件。
```

## LLM 兼容

当前使用 OpenAI-compatible `POST /v1/chat/completions`，适用于 LiteLLM、llama.cpp、vLLM、Ollama-compatible 网关以及其他兼容接口。

Reasoning 参数支持四种发送方式：

- `None`
- `OpenAI reasoning_effort`
- `Qwen chat_template_kwargs`
- `thinking parameter`

每个工具可以单独配置，例如：

- `analyze` → Medium
- `plan` → XHigh
- `review_diff` → Medium
- `document_repo` → Medium
- `update_docs` → XHigh

## 安全边界

LLM2MCP 0.1 默认只读，不提供任意 shell、写文件、Git commit 或 push 能力。

所有请求路径 canonicalize 后必须仍位于当前 workspace 内，symlink 越界同样会被拒绝。

默认排除常见敏感文件，例如：

- `.env*`
- `*.pem`
- `*.key`
- `id_rsa` / `id_ed25519`
- `credentials*`
- `secrets*`

需要注意：最终被选择用于分析的源码仍然会发送给你配置的 LLM API。请自行确认该 API 对源码数据的处理策略和访问控制策略符合要求。

## 项目状态

当前版本为 `0.2.0`。详细计划见 [ROADMAP.md](docs/ROADMAP.md)，架构见 [ARCHITECTURE.md](docs/ARCHITECTURE.md)。开发与发布相关说明统一放在 [DEVELOPMENT.md](docs/DEVELOPMENT.md) 和 [RELEASE.md](docs/RELEASE.md)。

## License

MIT
