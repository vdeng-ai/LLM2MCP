# 在 Cursor 中使用 LLM2MCP：接入独立的 AI 编码副模型

> 适用场景：希望继续使用 Cursor 自带的主模型与 Agent，但同时让 Cursor 调用自己部署的 Qwen、vLLM、llama.cpp 或其他 OpenAI-compatible API 做大范围代码分析、Debug、规划与 Review。

## 它解决的不是“替换 Cursor 主模型”

Cursor 已经支持部分 BYOK（自带 API Key）及自定义 OpenAI-compatible Base URL 的使用方式，**不能简单说“Cursor 完全不支持自定义 API”**。但 BYOK、指定模型、Agent、Tab 补全之间有功能差异；例如官方明确说明 Cursor Tab 继续使用内置模型。

LLM2MCP 不接管 Cursor 的 Chat/Agent/Tab 模型。它将你的独立 LLM 封装为本地 **stdio MCP 工具**：

```text
Cursor Agent（原来的主模型，负责决策与修改）
      ↓ MCP：传入任务描述 + 工作区相对路径
LLM2MCP（本地、只读、限制源码范围）
      ↓ OpenAI-compatible /v1/chat/completions
自托管或第三方 LLM（独立副模型）
      ↓ 分析 / 根因 / 计划 / 复审结果
LLM2MCP（校验、按需压缩）
      ↓ MCP 工具结果
Cursor Agent（只打开关键文件，执行最终修改）
```

这尤其适合“不愿或无法把任意自定义模型设置为主模型，但允许安装 MCP 工具”的 AI 编码环境。已有直连自定义 API 的用户同样可以用它做**上下文卸载与第二意见**。它**不会**让 Cursor Tab/自动补全使用副模型，也**不能**绕过 Cursor 自身的账号、功能或计费限制。

## 五步完成 Cursor 接入

1. 从 [Releases](https://github.com/vdeng-ai/LLM2MCP/releases) 下载 Windows、Linux 或 macOS 对应安装包，启动 LLM2MCP。
2. 在 **LLM API** 配置你的 **OpenAI-compatible API Base URL**（例如 `http://127.0.0.1:8000/v1`）、模型 ID 和必要的 API Key。副模型 API 必须能处理 **`POST /v1/chat/completions`**；只支持 Responses API 的端点不适用。使用 **刷新模型列表**确认模型名称，并点击 **测试连接**做真实推理与 stdio 检查。
3. 打开 **AI 编码智能体** 标签页，选择 **Cursor**，点击 **安装 / 更新**。应用会把当前可执行文件安装到稳定的用户级路径，再将 MCP 条目合并到 `~/.cursor/mcp.json`（Windows 对应用户目录中的 `.cursor\mcp.json`）；已存在的其他 MCP 项目会保留，配置修改前会生成 `*.llm2mcp.bak`。
4. 重载/重启 Cursor，在 **Customize → MCPs**（不同版本入口可能不同）确认 `llm2mcp` 服务器与工具已启用，并在 Cursor **Agent** 中允许 MCP 工具调用。
5. 在项目工作区直接向 Cursor 提问：

   > 修改前先调用 LLM2MCP 的 `analyze` 检查 `src/`，定位相关模块和关键源码范围。不要先读取整个目录。根据结果只打开需要的文件，再制定并实施修复。

**注意：** LLM2MCP 的 `mcp` 命令只读工作区，Cursor 主 Agent 才负责修改文件。若一开始就让 Cursor 读取整个仓库，再重复调用副模型，通常不能达到预期的上下文卸载效果。

## 手动配置（不能使用 GUI 安装时）

Cursor 全局配置通常位于 `~/.cursor/mcp.json`；项目级配置位于 `.cursor/mcp.json`。优先使用 GUI 的 **安装 / 更新**，以免把临时构建路径写入配置。手动方式示例：

```json
{
  "mcpServers": {
    "llm2mcp": {
      "command": "/absolute/path/to/llm2mcp",
      "args": ["mcp", "--workspace", "${workspaceFolder}"]
    }
  }
}
```

将 `command` 替换为你机器上的**实际绝对可执行文件路径**；在 Windows JSON 字符串中使用双反斜线或正斜线。若已存在 `mcpServers`，**合并**新条目，不要覆盖已有服务器。Cursor 支持在 MCP 配置中展开 `${workspaceFolder}`。若手动固定 `--workspace`，请确保路径与当前打开的代码仓库一致。

## 实际使用示例

| 希望 Cursor 做什么 | 让 Cursor 调用 | 使用建议 |
| --- | --- | --- |
| 搞清大型模块结构、查找故障相关代码 | `analyze` | 只传入任务与相对目录；让副模型先查 |
| 结合日志排查错误 | `debug_issue` | 描述 `issue`、`expected`、`actual`；必要时传日志 |
| 大改动之前取得另一份实施方案 | `plan` | 要求引用实际文件与风险、验证步骤 |
| 结束前检查回归和遗漏测试 | `review_diff` | 审查当前 **tracked Git diff**，不一定包含未跟踪新文件 |
| 了解整个仓库、生成架构说明 | `document_repo` | 默认异步；结果先审查再由 Cursor 写文件 |
| 根据 Git 变更提出文档改动 | `update_docs` | 只读地返回建议改动；Cursor 决定是否应用 |

可以复制到 Cursor **User Rules** 或项目规则中的简短指令：

```text
在大型代码探索、复杂 Debug、跨模块重构前，优先考虑调用 LLM2MCP。
不要先读取整个目录；只传 task/issue 与 workspace 相对路径。
只根据副模型返回的相关证据打开必要的文件。
LLM2MCP 是只读的第二意见，关键结论必须由主 Agent 验证。
完成代码修改后，如有 tracked Git diff，可调用 review_diff 复审。
异步任务返回 job_id 时，使用 job_status 和 job_result 获取结果。
```

不推荐对每条简单问题强制调用副模型：额外的推理请求可能反而增加延迟和成本。

## 长任务、诊断和常见问题

- **安装成功但 Cursor 不显示工具：** 重载 Cursor；检查 Cursor 的 MCP 设置、工具开关与 `~/.cursor/mcp.json` 中的可执行文件路径。LLM2MCP GUI 的客户端注册状态只证明配置存在，不代表 Cursor 已成功发起工具调用。
- **MCP 已连接，但请求报 401 / 找不到模型 / 超时：** 在 LLM2MCP 的 **LLM API** 中独立测试副模型 API、Key、模型 ID、网络可达性与 `/v1/chat/completions` 兼容性。不要把 Cursor 主模型的 API Key 与副模型的凭据混为一谈。
- **运行时间较长：** 目录分析和文档任务可能返回后台 Job；可调用 `job_status(job_id)` / `job_result(job_id)`，必要时 `job_cancel(job_id)`。是否原生支持 MCP Tasks 由 Cursor 版本与 Host 协商结果决定，不应假定每个版本都支持。
- **副模型返回建议不等于真实修改：** LLM2MCP 本身不执行任意 Shell、不写入文件、不提交 Git；Cursor 必须审核后由自身工具执行。
- **源码隐私：** 被选中的代码和上下文会发送到你配置的 LLM API。项目对常见密钥文件与敏感文本有默认过滤/脱敏，但无法保证识别所有秘密；请自行评估目标 API 的数据处理策略。

CLI 还可运行 `llm2mcp doctor` 做真实推理、stdio 和注册诊断；更多模型路由、日志与缓存说明参见 [RUNTIME.md](RUNTIME.md)。

## 其他同类 AI 编码工具

重点仍是 **Cursor**。类似场景也存在于部分原生模型选择受限、但能连接 MCP 的工具。例如 **Windsurf 的旧版 Cascade** 有官方 stdio MCP 配置接口，但其后继 Devin Local agent 使用不同的 MCP 配置路径；LLM2MCP 当前**没有为其提供专用一键安装**，如需尝试应使用 GUI 复制的通用 stdio MCP 模板、按所用版本的官方文档修改，并自行验证。不能把这种可配置性理解成已经完成产品级兼容测试。

Codex、Claude Code、GitHub Copilot CLI 等部分工具本身就支持不同程度的自定义 API / BYOK；在这些工具中，LLM2MCP 更适合作为**独立副模型与上下文卸载层**，而不是它们接入自定义主模型的唯一办法。

## 官方资料

- [Cursor：自带 API Key 与功能限制](https://cursor.com/help/models-and-usage/api-keys)
- [Cursor：MCP 配置与工具调用](https://cursor.com/docs/mcp)
- [Windsurf / Devin Desktop：旧版 Cascade MCP 配置说明](https://docs.devin.ai/desktop/cascade/mcp)
- [LLM2MCP：运行与诊断](RUNTIME.md)
