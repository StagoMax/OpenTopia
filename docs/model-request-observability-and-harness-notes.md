# 模型请求可观测性与本地 Harness 调研笔记

日期：2026-07-18

本文将已经验证的请求构造机制与从应用安装包中发现的线索分开记录。二进制文件中包含某个字符串，只能证明相关功能或模板存在，不能证明每次请求都会包含该字符串。

## OpenTopia 请求快照

现在，`AgentCore` 会在每轮调用 Provider 之前立即发出并持久化一个 `model_request` 事件。快照包含完整的逻辑 `ModelRequest`：

- `systemPrompt`
- 之前的 `conversation` 消息及其类型化内容片段
- 当前的 `userMessage` 和原生 `userContent`
- 模型可见的 `toolCandidates` 及其 JSON Schema
- 用于继续执行的历史工具调用和工具结果

该事件通过现有的 SQLite 事件存储写入，并可通过现有的任务事件 API 和 SSE 流读取。桌面端的轮次时间线会将每轮请求显示为可展开的 `Model request #N` 条目。

快照有意不包含 Authorization 请求头或 API Key。它记录的是与 Provider 无关的逻辑请求，而不是逐字节的 HTTP 抓包。OpenAI 兼容适配器随后会加入选定模型、temperature、Token 上限、reasoning effort 和流式选项，并将逻辑请求转换为 Chat Completions 的 `messages` 和 `tools`。Provider 返回 HTTP 400 后触发的兼容性重试，目前不会作为独立事件记录。

服务器直接发起的上下文压缩请求目前不会经过 `AgentCore`，因此不包含在每轮任务的请求事件中。

## 本机 ChatGPT 桌面应用证据

这里调查的是当前的 ChatGPT 桌面应用，而不是另外安装的 Codex CLI。它的 Windows 包仍保留旧身份 `OpenAI.Codex`，但当前产品界面已经是 ChatGPT：

- MSIX：`OpenAI.Codex_26.715.2305.0_x64__2p2nqsd0c76g0`
- Manifest 显示名：`ChatGPT`
- 可执行文件：`app\ChatGPT.exe`
- 安装包元数据：`codexAppBrand: "chatgpt"`，构建号 `5488`
- 从 Rollout 中观察到的桌面 Agent Runtime：`0.145.0-alpha.18`

本节没有使用单独安装的 `codex-cli 0.142.5` 作为证据。

### 桌面请求组装过程

对已安装的 `app\resources\app.asar` 进行静态检查后可以确认：桌面应用会先构造 `thread/start` 参数，再将执行交给内置 Agent Runtime：

1. 加载配置、账户和 Provider 数据、工作区状态以及 Git 状态。
2. 根据选定的任务模式请求动态工具。
3. 调用桌面 Host 操作 `developer-instructions`，传入已有的 developer instructions、cwd、任务 ID、Host ID、指令覆盖项，以及是否启用任务工具。
4. 使用返回文本替换 `thread/start` 中的 `developer_instructions`。

安装包中的构造器会将基础 Developer 文本与包裹在 `<app-context>` 中的内容合并，并按条件加入工作区依赖说明、任务工具说明、行文详细度说明、Heartbeat 指令和 Git 指令。默认桌面上下文自身包含以下部分：

- 图片、可视化与文件（Images/Visuals/Files）
- 工作区依赖（Workspace Dependencies）
- 自动化（Automations）
- 任务协调（Thread Coordination）
- 行内代码评论（Inline Code Comments）
- Git

随后，app-server 会将这些桌面指令与 Agent Runtime 的基础指令、World State 片段、历史记录、当前输入和模型可见工具合并。

### 已验证的当前桌面 Rollout

当前任务对应的本地 Rollout 为：

```text
~/.codex/sessions/2026/07/18/
rollout-2026-07-18T12-25-20-019f7378-aa2e-7ca3-bdf1-ebbbca5e8214.jsonl
```

其 `session_meta` 记录了 `originator: "Codex Desktop"`。内部的 `source: "vscode"` 标签是为兼容性保留的，并不表示该任务来自独立 CLI。在第一条普通用户消息之前，观察到了以下模型可见输入：

| 角色 | 片段 | 观察到的大小 |
| --- | --- | ---: |
| 基础指令 | Agent 个性和运行契约 | 16,299 字符 |
| `developer` | `<permissions instructions>` | 363 字符 |
| `developer` | `<app-context>` | 5,314 字符 |
| `developer` | `<collaboration_mode>` | 977 字符 |
| `developer` | `<plugins_instructions>` | 1,014 字符 |
| `developer` | `<skills_instructions>` | 15,117 字符 |
| `developer` | 主 Agent 与团队协作指令 | 2,183 字符 |
| `developer` | `<multi_agent_mode>` | 271 字符 |
| `user` | `<environment_context>` | 471 字符 |

Rollout 的 `world_state` 还分别包含 `agents_md`、应用指令、环境、环境指令、Host Skills、插件指令和 Skills。`turn_context` 记录 cwd、工作区根目录、日期、时区、审批策略、沙箱和权限配置、模型、reasoning effort、个性、协作模式以及多 Agent 模式。

桌面会话注册了动态工具 `codex_app`。标准内置工具和插件工具的 Schema 会由内置 Runtime 为每次推理请求组装。后续历史记录还会加入 Assistant 消息、Reasoning 项、工具调用、工具输出、上下文变化以及之后的用户和 Developer 输入。

因此，ChatGPT 桌面应用提交给模型的内容并不只有输入框文本。它由以下部分组成：基础 Agent 契约、桌面专用 Developer 指令、当前 World State 和 Turn State、持久化的仓库和用户指引、选中的 Skills 与插件、对话和工具历史、当前用户输入，以及模型可见的工具 Schema。Rollout 是可重放的模型上下文记录，但不是逐字节的 TLS 请求抓包，也不包含 Authorization 请求头。

## 本机 Trae 证据

在本机观察到的安装版本：

- Trae CN：`3.3.72`，安装在 `J:\Trae CN`
- TRAE Work/SOLO CN：`0.1.36`，安装在 `D:\Software\TRAE SOLO CN`

Trae 的主要编程 Agent 是原生模块，而不是可直接阅读的 JavaScript：

```text
J:\Trae CN\resources\app\modules\ai-agent\ai_agent.dll
D:\Software\TRAE SOLO CN\resources\app\modules\ai-agent\ai_agent.dll
```

两个二进制文件的 SHA-256 Hash 不同。从其中嵌入的符号和日志目标可以确认以下请求构造阶段和输入：

- `ChatPromptBuilder`、`ChatPromptBuilderImpl` 和 `build_llm_prompt`
- `rs_03_get_history_message`、`rs_06_resolver_user_message` 和 `rs_13_render_user_prompt`
- 自定义 Agent 使用的 `agentName`、`systemPrompt` 和 `whenToUse`
- 用于标题、图标、项目、分支、Pull Request、输入优化和自定义 Agent 生成的具名 System Prompt 模板
- 模型可见的工具调用和结果、`custom_tools`、浏览器、MCP 与 Skill 工具，以及工具结果裁剪和压缩控制

可读的 Workbench 层确认了以下持久化规则注入机制：

- `.trae/rules/` 下的工作区规则
- `project_rules.md`
- 产品数据目录 `user_rules/` 下的用户规则，以及旧版 `user_rules.md`
- 默认启用 `AGENTS.md` 导入
- 可选导入 `CLAUDE.md` 和 `CLAUDE.local.md`
- `alwaysApply`、按文件匹配、由模型决定和手动启用等规则模式

Trae 的行内补全子系统与 Agent 相互独立。其 Prompt 模板在 FIM 标记周围包含编辑历史、检索到的代码片段、符号、RAG 上下文、文件路径、前缀和后缀。这只能作为补全请求的证据，不能证明 Chat Agent 使用了相同的 System Prompt。

本机 `ai-agent` 的 stdout 日志会暴露阶段名和路由名，但不包含字面量 `systemPrompt` 条目。日志展示了 Prompt 构造阶段、历史和用户消息解析、请求路由、模型详情查询、工具、自定义工具、MCP 以及工具结果提交。仅凭这些日志无法可靠还原 Trae 完整的基础 System Prompt；它可能嵌入在本地程序中、由远程配置选择，或由本地配置和服务端配置共同组装。

## 应持续保持可见的 Harness 分层

对于 OpenTopia，请求可观测能力应当能够区分以下各层：

1. 基础 Agent 或 Harness 指令以及策略边界。
2. 工作区、沙箱、权限、时间和模型上下文。
3. 用户级和仓库级持久化指令。
4. 对话历史、压缩摘要和当前用户消息。
5. 选中的 Skills、插件、MCP 资源和其他注入上下文。
6. 工具名称、描述、JSON Schema、调用和结果。
7. Provider 生成参数和最终传输负载。

当前实现的逻辑快照覆盖了 `AgentCore` 各轮请求的第 1 至第 6 层。若要捕获第 7 层以及服务器直接发起的模型调用，应当实现 Provider 传输观察器，而不是将密钥写入应用日志。
