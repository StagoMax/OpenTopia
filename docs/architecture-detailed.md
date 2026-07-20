# OpenTopia 架构教程（以当前实现为准）

> 这是一份面向学习的架构教程：先解释“为什么要这样拆分”，再说明代码“如何实现”。它以当前仓库源码为准，不把设计灵感、后续计划或某个依赖库的潜在能力表述为 OpenTopia 已具备的功能。
>
> 主要对应目录：`apps/desktop/`、`crates/opentopia-server/`、`crates/opentopia-core/` 与 `crates/opentopia-cli/`。HTTP 路由以 `crates/opentopia-server/src/main.rs` 的 `build_router` 为准。

---

## 0. 如何学习这份架构

阅读架构时，不要先背文件名。对每一个模块都问四个问题：

1. **它接收什么输入，产生什么输出？** 这能帮助你画出数据流。
2. **为什么不把它放到别的模块？** 这能看出边界和职责。
3. **它失败、刷新页面或重启进程后会怎样？** 这能看出可靠性设计。
4. **谁有权限调用它？** 这能看出安全边界。

建议按下面的顺序阅读：

1. 先读第 1、5 和 15 节，建立“一条消息如何变成一次 Agent 执行”的全景。
2. 再读第 2、3、4 节，理解为什么 UI、服务端、数据库必须分开。
3. 然后读第 7、8 节，理解“模型能做什么”不等于“模型应该被允许做什么”。
4. 最后按需要阅读 MCP、浏览器、预览、XLSX 和子智能体等扩展能力。

### 0.1 先认识几个词

| 术语 | 通俗解释 | 在本项目中的意义 |
|---|---|---|
| 进程 | 操作系统中独立运行的程序 | Electron、React 所在的渲染进程和 Rust 服务端不是同一个进程 |
| API | 模块之间约定的“请求格式和返回格式” | React 用 HTTP 调 Rust 服务端，而不是直接摸数据库 |
| 状态 | 系统当前记住的事实 | 线程、消息、正在运行的 Turn、审批、终端历史等 |
| 持久化 | 把状态写到磁盘，重启后仍能恢复 | SQLite 保存本地会话与执行记录 |
| 事件流 | 按时间顺序不断推送发生过的事 | SSE 把模型文本、工具调用和审批变化推给 UI |
| 策略 | 在执行前判断“是否应允许” | `Allow`、`Ask`、`Deny` 三种决定 |
| 沙箱 | 操作系统层面对进程实际能碰到什么的限制 | 限制文件根目录、网络和子进程权限 |
| Token | 用来证明调用方身份的一段随机值 | 防止其他本机网页或进程随意调用本地 API |

### 0.2 用一个例子建立心智模型

假设用户输入：“检查项目中的构建错误并修复。”系统不是把这句话直接交给一个拥有所有电脑权限的模型，而是经过下面这条链：

```text
用户输入
  -> UI 发请求给服务端
  -> 服务端保存“用户说了什么”，创建一次 Turn
  -> Agent 请求模型思考
  -> 模型提出“读取文件 / 执行命令 / 修改文件”等工具调用
  -> 策略和沙箱检查是否允许
  -> 工具执行，结果再交回模型
  -> 模型给出最终回答
  -> 过程中的事件和最终结果保存到 SQLite，并实时显示到 UI
```

这样设计的目的有三个：**可控**（危险动作可拦截）、**可观察**（用户能看到正在发生什么）、**可恢复**（刷新或重启后不把已发生的工作忘掉）。后续章节都是在解释这条链中的某一段。

---

## 1. 架构概览：为什么要分层

OpenTopia 是本地优先的桌面 Agent 工作台。它不是一个单体 Electron 应用，而是由五个可替换的运行边界组成：

| 边界 | 实现 | 职责 |
|---|---|---|
| 桌面壳 | Electron 主进程 | 窗口、启动本地服务、密钥安全存储、文件选择、系统打开、日志和可见浏览器宿主 |
| 渲染层 | React + TypeScript | 项目/线程工作台、消息和事件展示、审批、终端、预览及设置界面 |
| 本地 API | Axum | 鉴权后的 REST/SSE 接口、Turn 调度、终端和 PTY 管理、持久化编排 |
| Agent 核心 | Rust core crate | 模型工具循环、策略判定、执行环境、MCP、浏览器、子智能体、预览和表格逻辑 |
| 本地状态 | SQLite | 项目、线程、消息、事件、Turn、审批、产物、MCP 配置、终端记录及应用设置 |

CLI（`crates/opentopia-cli`）是另一入口：它复用核心模型和 SQLite 会话，而不是绕过桌面端另建一套业务逻辑。

**为什么不把这些都写进 Electron 或一个 Rust 程序？** 因为它们的风险和变化速度不同：界面需要快速迭代；命令执行和文件写入需要严格限制；持久化需要在重启后稳定；模型接入又可能频繁更换。把它们拆开以后，修改 UI 不会直接破坏文件权限，替换模型也不必改窗口代码。

```text
Electron main process
  ├─ BrowserWindow / preload bridge
  ├─ secure secret storage / desktop browser broker
  └─ starts or reuses opentopia-server

React renderer -- Bearer token over HTTP/SSE --> Axum local server
                                                   ├─ SQLite session store
                                                   ├─ AgentCore + provider + tools
                                                   ├─ policy + execution environment + sandbox
                                                   ├─ MCP stdio host / browser runtime / subagents
                                                   └─ event and terminal broadcast buses
```

桌面端和服务端是独立进程。渲染层不直接调用 Node.js、SQLite 或 Rust 内部 API；它只能访问 preload 暴露的受限桌面能力，以及带认证的本地 HTTP API。

**可以这样理解职责：** React 负责“让用户看见和操作”；服务端负责“判断、执行、记录”；核心库负责“可复用的规则和能力”；Electron 负责“只有桌面应用才能做的事”。这叫作**关注点分离**，目标不是制造更多文件，而是让高风险能力集中在更容易审查的位置。

---

## 2. 进程启动与桌面边界

**这一层要解决的问题：** 网页界面擅长展示和交互，却不应该天然拥有读取磁盘、启动进程或保存密钥的权限。Electron 将“像网页一样的界面”和“本机桌面能力”放在不同进程，并用窄桥连接它们。

学习时可抓住一个原则：**权限应该从少到多逐层增加，而不是从多到少再试图收回。** 渲染进程从零权限开始；preload 只开放具体动作；主进程再决定是否执行；真正的工作区变更仍转交 Rust 服务端。

### 2.1 Electron 主进程

`apps/desktop/electron/main.cjs` 负责桌面生命周期。

- 创建主窗口，并通过 preload 启用 `contextBridge`；渲染进程没有直接的 Node.js 权限。
- 启动前探测本地服务的 `/health`。若服务不可用，才启动随应用携带或开发环境中的 `opentopia-server`；退出时清理由本进程启动的子进程。
- 为每次 Electron 启动生成随机 API Token，并以 `OPENTOPIA_API_TOKEN` 注入服务端。Token 不提供给任意网页或外部来源。
- 导入项目 `.env` 和兼容别名，准备 Rust/MinGW 与 Windows Codex sandbox 所需环境；这些是开发和桌面启动辅助，不是 Agent 的权限豁免。
- 写入 JSONL 启动/崩溃日志。日志写入前会按密钥名、Bearer 值和常见 API Key 形态进行脱敏。
- 管理最近工作区、系统文件夹选择和外部文件/链接打开。
- 启动可见浏览器宿主时，同时启动仅回环可访问、带随机 Token 的 broker，供 Rust 端共享操作 Electron 页面。

开发模式和打包模式使用相同的服务端协议；区别仅在服务端二进制的解析和加载来源。生产更新逻辑位于 `electron/updater.cjs`，目前是打包后的更新骨架，签名、发布和公证不是该代码库已经完成的发布流程。

### 2.2 Preload Platform Bridge

`apps/desktop/electron/preload.cjs` 只暴露白名单 IPC：

| 分类 | 暴露能力 |
|---|---|
| 平台 | `getPlatformInfo`、`openExternal`、`openPath` |
| 工作区与上下文 | 选择工作区/上下文文件、读取/写入/删除最近工作区 |
| 密钥 | 列出密钥来源元数据、设置、删除；不提供“读取明文密钥”接口 |
| 日志 | 列出日志、按偏移量读取日志 |
| 浏览器宿主 | 创建、显示、隐藏、导航、前进后退和观察 `WebContentsView` 状态 |

因此，`window.opentopia` 是能力桥而不是通用 IPC 通道。普通浏览器模式下 `apps/desktop/src/platform.ts` 提供降级实现，但浏览器模式并不具备 Electron 的密钥、原生窗口或路径打开能力。

### 2.3 API Key 的存放和注入

Electron 使用 `safeStorage` 加密 `userData` 下的密钥记录。渲染层只能写入、删除或获取“是否已配置”等元数据。主进程在启动服务端时，只有在显式环境变量和 `.env` 都没有提供 Key 的情况下，才将已解密的 provider Key 作为 `OPENTOPIA_API_KEY` 传给该服务端子进程。

这意味着：安全存储降低了本地静态明文泄漏面，但服务端进程运行期间仍需在其环境中使用 Key；它不是远程密钥托管系统。

**设计目的：** 将“用户输入密钥的界面”和“拿到明文密钥的代码”隔离开。即使前端遭到普通 XSS 风险，也没有一个 `getSecret()` 接口可直接把 Key 交出去。这是安全设计里常说的**最小暴露面**。

---

## 3. 本地 API、鉴权与事件流

**这一层要解决的问题：** Electron 中仍可能加载网页内容，开发时也可能有浏览器访问本机端口。只要有 HTTP 服务，就不能因为它叫“localhost”而默认信任所有调用方。

### 3.1 本地 API 的认证

所有路由都经过 `auth::authorize`，包括 `/health` 和 SSE：

- 服务启动要求 `OPENTOPIA_API_TOKEN`，且 Token 至少 32 字节。
- 客户端必须发送 `Authorization: Bearer <token>`；比较使用常量时间比较函数。
- 浏览器请求还会检查 Origin。允许打包应用的 `file:`/`null` 来源和配置的回环开发来源；非本机 Web Origin 会被拒绝。
- CORS 仅允许 `GET`、`POST`、`PATCH`、`PUT`、`DELETE` 及认证/内容类型相关请求头。

服务默认监听 `127.0.0.1:8787`。这是一项本地进程认证设计，并不等价于可安全暴露到局域网或公网。

**为什么健康检查也要 Token？** 如果 `/health` 例外，通常会慢慢出现更多“为了方便”的例外，最终让安全边界变得不可推理。这里统一要求 Token，使规则变成简单的一句话：任何 API 调用都先认证。

### 3.2 路由分组

| 分组 | 主要接口 | 说明 |
|---|---|---|
| 健康与设置 | `/health`、`/api/settings`、`/api/provider/*` | 服务可用性、Provider 设置、健康和连接测试 |
| 项目与线程 | `/api/projects`、`/api/threads`、`/messages` | 项目/线程 CRUD、发送消息、读取消息 |
| Turn 与事件 | `/turn`、`/turn/cancel`、`/events`、`/events/stream` | 当前/最近 Turn、取消、历史事件和 SSE |
| 审批 | `/approvals`、`/approvals/:id/decision` | 查看审批和允许/拒绝后的续跑 |
| 工作区 | `/workspace/tree`、`/file`、`/diff`、`/diff/revert`、`/diff/hunk` | 文件树、只读预览、Git diff 与受控变更操作 |
| 终端 | `/terminal/commands`、`/terminal/stream`、`/terminal/session/*` | 一次性命令、持久 PTY 会话和流式输出 |
| 扩展与运行时 | `/mcp/*`、`/browser`、`/sandbox`、`/context`、`/skills` | MCP、浏览器、沙箱描述、上下文与 Skill 目录 |
| 产物与预览 | `/artifacts`、`/previews/*`、`/trajectory` | 产物、二进制/表格预览和线程轨迹导出 |
| Git 与 Agent 协作 | `/git`、`/subagents/*` | 受控 Git 工作流、Agent 创建、消息、追问、等待和打断 |

接口的完整方法组合、参数和返回模型应以 `main.rs` 为准；上表用于理解职责边界，而不是替代 API 契约。

### 3.3 SSE：持久化历史加实时通知

Agent 事件先写入 SQLite，再发布到进程内 `EventBus`。`GET /api/threads/:thread_id/events` 读取历史事件；`/events/stream` 将历史回放和实时广播拼成同一 SSE 流，并支持 `since` 序号增量同步。客户端应把 `seq` 作为去重和断线恢复依据，而不是假设 SSE 永不丢失。

终端输出使用独立的 `TerminalBus` 和 `/terminal/stream`。它与 Agent 事件流分离，避免把 shell 的字节流混入 Agent 生命周期事件。

**为什么不用“请求完成后一次性返回结果”？** Agent 运行可能持续数分钟，也可能等待用户审批。SSE 让 UI 在过程中看到“模型正在输出”“工具正在执行”“需要批准”等状态。更重要的是，事件先保存到 SQLite：网络断开只会失去实时通知，不会丢失已经发生的事实。这个组合叫作“**持久化日志 + 可重连订阅**”。

---

## 4. 持久化领域模型

**这一层要解决的问题：** 如果状态只存在 React 内存或 Rust 内存，刷新窗口、崩溃或重启后，一次执行就会变成无法解释的黑盒。数据库将“发生过什么”变成可查询的事实。

`SqliteSessionStore` 是服务端唯一的会话持久化入口。它建表和迁移旧 schema，服务启动时还会把遗留的运行中 Turn 标记为 `interrupted`，把未完成子智能体标记为失败，防止 UI 将进程重启前的任务误认为仍在执行。

主要数据关系如下：

```text
Project 1 ── * Thread 1 ── * Message
                     ├── * AgentEvent (按 seq 排序)
                     ├── * TurnRecord
                     ├── * Approval ── 0..1 ApprovalContinuation
                     ├── * Artifact
                     ├── * TerminalCommandHistory
                     └── * SubagentRun

AppSettings 1 ── * McpServer
Thread * ── * McpServer (thread_mcp_servers)
```

| 实体 | 关键作用 |
|---|---|
| `Project` / `Thread` | 工作区的项目归属、线程标题、固定/归档等 UI 所需状态 |
| `Message` | 用户和助手消息；消息部分可包含文本、选中的上下文源和 Skill 引用 |
| `AgentEvent` | 附带线程、可选 Turn 和顺序号的事件审计记录 |
| `TurnRecord` | 一个用户请求的执行状态：`running`、`waiting_approval`、`cancelling`、`succeeded`、`failed`、`cancelled`、`interrupted` |
| `Approval` | 待决动作、原因和最终状态；续跑所需的 `AgentContinuation` 单独持久化 |
| `Artifact` | 线程范围内的 inline 文本或文件路径产物；大工具输出可变为产物而非无限塞进消息 |
| `TaskPlan` / `ContextSummary` | 可恢复的任务计划与上下文压缩摘要 |

SQLite 解决的是本地可恢复性和审计，不是多用户并发数据库。服务端用 `TurnManager` 限制一个线程同时只有一个活动 Turn；数据库也有相应的活动 Turn 唯一索引作为第二层约束。

**为什么既有 `TurnManager` 又有数据库唯一索引？** 这是典型的双层保护：内存管理器负责快速协调当前进程；数据库约束在并发竞争或未来代码改动时兜底。只靠前者，重启后信息会消失；只靠后者，错误反馈和取消管理会变得笨重。

---

## 5. 从发送消息到最终回答

**这一节是整份文档的核心。** `Turn` 可以理解为“用户一次请求对应的一次可追踪执行”。不要把它和一条消息混为一谈：一条用户消息会创建一个 Turn，但一个 Turn 内可能发生多次模型调用、工具调用、审批暂停和恢复。

### 5.1 创建 Turn

`POST /api/threads/:thread_id/messages` 执行以下步骤：

1. 校验线程存在，拒绝空消息（除非附带上下文源或选中的 Skill）。旧式 `/run`、`/read` 直接工具命令会被拒绝，用户应使用工作区/终端 API 或正常 Agent 请求。
2. 规范化并加载所选上下文源；发现工作区可用 Skills，将用户选中的 Skill 作为消息引用保存，并把受大小限制的正文注入当前 Turn。
3. 若存在待决审批则拒绝新请求，避免两个执行链竞争同一线程。
4. 由 `TurnManager` 原子地开始 Turn，持久化用户消息。
5. 在 Tokio 后台任务中运行 Agent；HTTP 请求立即返回已保存的用户消息。

服务端组装模型输入时会带上近期对话、最新上下文摘要、用户附加源的内容、显式选择的 Skill 正文和可用工具目录。显式选择只影响当前 Turn，历史消息保留 `SkillRef` 而不会静默重读可能已经变化的文件；未选择的 Skill 仍通过 `list_skills` 和 `read_skill` 渐进加载。

**为什么先保存消息再后台运行？** 这样 API 可以迅速确认“请求已收到”，而长时间模型调用不会占住 HTTP 请求。即使随后模型调用失败，用户的原始请求、失败事件和 Turn 状态仍可被诊断和重试。

### 5.2 Agent 工具循环

`AgentCore::run_turn_detailed_streaming` 的职责是将 Provider、工具、策略和事件串起来：

```text
TurnStarted
  -> Provider SSE 文本/工具调用 delta
  -> ToolCallStarted
  -> 策略判定 + 执行环境执行
  -> ToolCallFinished / ToolCallFailed
  -> 继续 Provider（携带工具结果）
  -> AssistantMessage + TurnCompleted
```

Provider 首轮没有工具调用时，直接落库助手消息并结束。否则 Agent 在每一轮把模型工具调用转为 `ToolCall`，顺序执行后将结构化 `ToolResult` 回传给 Provider。大文件、搜索和 shell 输出会在满足阈值时保存为 Artifact，并在工具结果中返回引用。

Agent 不会在每个 Turn 开始时自动执行 `list_files`，也不会承诺“先读文件再改文件”的固定工作流。是否调用工具由模型和系统提示共同决定。

**这里的逻辑分工很重要：** 模型负责提出下一步；工具负责做确定性的操作；服务端负责让这两者之间有记录、有权限检查、有超时。模型不是直接执行 PowerShell 的主体，而是“提出工具调用请求”的决策者。

### 5.3 模型控制的循环终止与 rollout budget

Agent Turn 不使用固定工具轮数或总运行时长来判断任务是否完成。每当模型返回一个或多个工具调用，运行时执行工具、追加结构化结果并再次请求模型；模型可以根据新观察修改计划、增加步骤、换一种方法或继续验证。只有模型返回不含工具调用的助手答复时，当前 Turn 才成功结束。

`update_plan` 和 `complete_task` 都是普通工具。它们把计划或结构化完成信息写入可恢复状态，但工具调用本身不会终止 Turn；工具结果仍会回到模型，模型必须再返回最终助手答复。运行时也不按“同一个调用重复了几次”猜测任务是否陷入死循环，因为同一检查在环境变化后可能是必要操作。

无限循环和成本失控由可选的 rollout token budget 约束。预算按 Provider 报告的输出 token 和未缓存输入 token 加权累计；达到 25% 和 10% 剩余额度时向模型注入提醒，额度耗尽后不再发起下一次模型请求。预算默认关闭，可在 Provider 设置中按模型价格和任务规模启用。单个工具仍有自己的超时，用户也可以随时取消 Turn。

上下文窗口是另一条独立边界。工具历史接近窗口阈值时，运行时把较早的已完成调用压缩为摘要，并保留原始目标、持久计划、近期工具结果和验证信息。这样动态增加步骤不依赖模型永久记住所有原文，而依赖“当前目标 + 可恢复计划 + 压缩后的已知事实”持续回注。

**为什么不使用固定轮数？** 轮数无法区分长程任务和死循环；一次搜索可能解决问题，复杂迁移也可能合理地运行几十轮。无工具最终答复提供清晰的协议终点，加权预算负责资源上限，计划和摘要负责目标连续性，三者职责互不混淆。

### 5.4 取消与审批续跑

取消 `/turn/cancel` 触发 `CancellationToken`，用于中断 Provider 流或工具 future；最终状态由 Turn 管理器写回。

当策略返回 `Ask` 时：

1. Agent 写入待决 `Approval`，持久化当前 Provider 对话、已完成工具结果、待执行调用、自动压缩摘要和 rollout budget 状态。
2. Turn 转为 `waiting_approval`，并发送 `ApprovalRequested` / `TurnSuspended` 事件。
3. 用户调用审批接口。允许只授予这次待决调用；拒绝会作为结构化工具错误回传给同一个模型对话。
4. 服务端新建恢复 Turn，使用已持久化 continuation 继续，而非重新开始整个请求。

浏览器面板的域名授权是一个特例：它只记录域名授权，不隐式重放先前导航。用户或模型必须再次发起明确操作。

**为什么审批要保存 continuation？** 如果只保存“用户点了允许”，系统已经忘了模型当时想调用什么工具、前面得到了哪些结果。保存 continuation 后，允许或拒绝都能回到同一段推理链继续，而不是重新问模型一次并产生不同结果。

---

## 6. Provider 与上下文管理

**这一层要解决的问题：** 应用业务不应该绑死在某个模型厂商、URL 或工具消息格式上。Provider 抽象把“应用想让模型做什么”与“某个 API 要怎么调用”分开。

### 6.1 Provider 抽象

`ModelProvider` 定义 `complete`、`stream` 和健康检查。默认运行时使用：

- `OpenAiCompatibleProvider`：OpenAI Chat Completions 风格的流式 SSE、文本增量、工具调用增量和使用量解析。
- `MockProvider`：没有可用真实 Provider 配置时的本地替代实现。

`AppSettings` 可以保存多个 Provider，并由 `active_provider_id` 选择当前 Provider。每个真实 Provider 配置包含类型、base URL、模型名和 API Key 的环境变量来源。更新设置后，服务端会重建使用新设置的 `AgentCore`。

对于工具结果历史被部分兼容网关拒绝的情形，OpenAI-compatible Provider 仅在收到 HTTP 400 且请求确实含工具结果时，尝试一次紧凑兼容格式的重试；其他错误不做无限重试。

**这体现了一个可靠性原则：重试必须有条件、有上限。** 没有条件的重试会把临时故障变成无限请求；这里仅针对已知的兼容性问题，且只重试一次。

### 6.2 上下文源和摘要

`context_sources.rs` 在服务端加载用户显式选择的文件，执行路径规范化、类型/大小限制并产生适合模型的内容部分。它不是对工作区做全量索引。

上下文预算按近似 token 数量累计。服务端支持手动和自动压缩：摘要及覆盖范围持久化为 `ContextSummary`，后续模型请求注入最新摘要并保留有限近期消息。若 Provider 不能执行摘要或预算不足，调用会返回可见错误，而不是伪造摘要。

**为什么不能把所有历史都发给模型？** 模型的上下文窗口有限，历史越长成本越高，重要信息也越容易被淹没。摘要是“有损压缩”：牺牲部分原文细节，保留后续任务需要的结论。因此摘要必须保存覆盖范围，才能知道它替代了哪些旧内容。

---

## 7. 权限、执行环境与沙箱

**这一层要解决的问题：** “模型说要执行”只是一个建议，不能自动转换为“操作系统允许执行”。OpenTopia 将决策、实际执行和操作系统隔离分开，避免单点失效。

这三个层次必须分开理解：

| 层次 | 负责什么 | 典型结果 |
|---|---|---|
| `BasicPolicyEngine` | 请求是否可读、可写、可执行、可访问网络/MCP | `Allow`、`Ask`、`Deny` |
| `LocalExecutionEnvironment` | 将逻辑路径限制在工作区，执行读写、补丁、命令或 stdio 进程 | 解析后路径、stdout/stderr、超时和取消结果 |
| `LocalSandboxConfig` | 对子进程增加 OS 级命令包装、根目录和网络限制 | disabled/best_effort/enforce 等实际隔离策略 |

### 7.1 权限模式

配置支持 `chat`、`read_only`、`auto`、`approve`、`full_access`。具体结论还取决于命令规则、工作区路径、MCP 工具描述和网络策略，因此不应将任一模式简单描述为“永远允许”或“永远询问”。

策略检查在工具执行之前。读写工具检查目标路径，shell 与 patch 检查命令，MCP 通过工具注解评估风险；返回 `Ask` 时才进入审批 continuation。

### 7.2 本地执行环境

执行环境处理的核心不变量：

- 读取现有路径时 canonicalize，并验证其仍位于工作区根目录下。
- 写入路径逐段检查，拒绝利用 `..` 或符号链接逃逸工作区。
- `ExecRequest` 可限制 cwd、环境、stdin、超时、输出大小和取消 Token。
- `apply_patch` 最终调用受控的 `git apply --whitespace=nowarn -`；它不是对任意系统路径的写权限。
- 交互式终端使用 `portable-pty`，与一次性 shell 命令分开管理。

### 7.3 OS 沙箱

沙箱配置包含文件系统根、网络策略和执行环境类型。不同平台使用不同 wrapper：Linux 使用 `bwrap`，macOS 使用 `sandbox-exec`，Windows 使用 Codex restricted-token helpers。`best_effort` 在本机缺少必要工具时可以退回；`enforce` 要求隔离建立成功；`danger_full_access`/`disabled` 则弱化或关闭该层。

因此，权限策略可拒绝一个操作，沙箱也可能在策略已允许后阻止子进程；二者都需要保留，不能互相替代。

**可以用两道门来理解：** 策略门问“按产品规则，这次操作该不该做”；沙箱门问“即使要做，操作系统实际允许它碰到哪些资源”。第一道门便于向用户解释和审批，第二道门用于在实现出错或命令被绕过时继续兜底。

---

## 8. 内置工具与工作区能力

**这一层要解决的问题：** 模型擅长决定“需要查什么、改什么”，却不擅长可靠地直接操作文件、进程或 Git。工具把这些操作变成有 schema、有返回值、可审计的程序接口。

`ToolRegistry::with_builtins()` 注册以下一等工具：

| 工具 | 用途 |
|---|---|
| `list_files`、`read_file`、`write_file` | 工作区内的目录和 UTF-8 文本文件操作 |
| `search` | 首选 `rg`，不可用时使用受限的文本扫描回退 |
| `shell` | 带超时、输出截断、取消和策略检查的 shell 命令 |
| `git_diff`、`apply_patch` | 获取 diff、用 `git apply` 应用统一补丁 |
| `update_plan`、`complete_task` | 持久化计划和结构化完成信息；工具调用本身不会终止 Turn |
| `list_skills`、`read_skill` | 发现并按需读取 `SKILL.md` |
| `browser` | 以 action 执行导航、快照、点击、输入、等待、截图和下载 |
| `spreadsheet` | 检查、列 Sheet、读区间、创建或更新 XLSX |
| `spawn_agent`、`send_message`、`followup_task`、`interrupt_agent`、`list_agents`、`wait_agent` | Codex 风格 Agent Thread、mailbox 与生命周期控制；旧工具名保留兼容 |
| `<server>__<tool>` | 运行时同步的 MCP 工具包装器 |

工具调用不是 HTTP API 的旁路。所有内置读写和命令工具都带 `ToolContext`，其中包含工作区、策略、执行环境、取消 Token、线程、存储、浏览器和子智能体调度器。

**学习工具设计时可以观察三个要点：** 输入有明确 schema（模型不能随意猜参数）；执行有明确上下文（工具知道自己在哪个工作区、谁发起、何时取消）；输出既有文本也有结构化 metadata（UI、模型和数据库都能使用）。

### 8.1 工作区和 Git UI API

工作区 HTTP 接口提供文件树、单文件读取、staged/unstaged diff、文件 revert 和 hunk stage/unstage/discard。路径解析在服务端完成，前端不应把任意绝对路径当作可信输入。

另有 `/api/threads/:thread_id/git` 的 Git Workflow，支持状态、分支列举/创建/切换、提交、推送、比较和 worktree 创建等明确动作。该 API 经执行环境和沙箱运行 Git，不承诺具备 GitHub PR 创建或远程凭证管理能力。

### 8.2 终端

终端分为两种会话：

- 一次性命令：执行、取消、历史和 SSE 输出；历史持久化在 `terminal_history`。
- 持久 PTY：按线程维护 shell、输入、尺寸调整和关闭；UI 通过 xterm.js 与它交互。

终端是用户显式操作面，Agent 的 `shell` 工具是模型工具面。二者共享工作区与安全边界，但事件流和生命周期不同。

**为什么不复用同一个“命令执行接口”？** 用户终端需要交互、持续输入和模拟终端尺寸；Agent shell 需要受控超时、简洁结果和模型可读输出。外表都像“运行命令”，但交互模型不同，拆开反而更简单。

---

## 9. MCP、Skills 与子智能体

**这一层要解决的问题：** Agent 的核心能力应保持小而稳定；外部工具、领域说明和并行任务则应按需接入。MCP、Skills、子智能体分别解决“接入能力”“告诉模型如何做”“并行完成独立工作”。

### 9.1 MCP

MCP 配置（命令、参数、cwd、环境变量名、超时、启用状态）持久化在 SQLite。`McpExtensionHost` 通过受控 stdio 进程启动 MCP server，完成初始化、工具列表获取和 JSON-RPC 调用。应用启动会后台恢复已启用服务，创建、更新、线程启用和 Agent 首次使用也都会经过串行化的 `ensure_server` 入口。

工具名称按 `<server>__<tool>` 转换为公开名称，并拒绝跨服务的名称冲突。线程可单独启用或禁用某个 MCP server；Agent 在运行前只同步该线程启用且已经 ready 的工具。MCP server 进程也由执行环境工厂创建，因此仍受本地 sandbox 配置约束。

MCP 是对本地 Agent 能力的扩展，不是自动信任边界。MCP 工具调用仍经过 Policy Engine 的工具风险判断。

**为什么 MCP 要用独立 stdio 进程？** 扩展可以崩溃、卡住或升级，而核心服务仍应保留自己的状态。进程边界配合超时让外部扩展不会直接变成核心内存的一部分。

### 9.2 Skills

Skills 从用户目录和工作区的 `.codex/skills/` 发现，使用 `SKILL.md` 的前置 YAML 元数据生成 `SkillDescriptor`。读取时有大小限制和截断标识，避免把无界文档直接塞入模型上下文。

用户在消息中显式选择 Skill 时，服务端保存引用并把受限正文注入当前 Turn；模型也可以通过 `list_skills` 和 `read_skill` 自主选择未固定的 Skill。这将“用户固定的上下文”与“模型按需加载的指令”区分开来。

### 9.3 Agent Thread 与直接协作

`SubagentScheduler` 是确定性的 Agent Control bridge，不是业务工作流引擎。模型自行判断是否委派、选择 Profile、复制多少父历史、何时通信或等待；运行时负责稳定身份、隔离、限额和状态。

- 每棵任务树使用 `/root/...` 规范路径，树内 Agent 可按路径或 UUID 直接通信；
- `send_message` 只投递 mailbox，`followup_task` 会在空闲 Agent 上启动新回合；
- 完成的 Agent 保留身份和对话，可使用同一 ID 继续工作；
- `default`、`worker`、`explorer` 及 `.codex/agents/*.toml` 提供可发现 Profile；
- 子 Agent 继承权限和沙箱，Profile 只能收紧安全边界；
- 默认最多 6 个活动线程、最大派生深度 1，父 Turn 取消时递归取消后代；
- 状态通过 SQLite 和广播事件投影到父线程与桌面 UI。

Agent 的完成、失败、取消或超时不等价于根任务完成。根 Agent 需要检查结果和错误后再综合。并发只适合输入自包含、输出可独立验收、写集合不相交的工作；有依赖时应显式顺序执行或通过消息传递前置结果。

---

## 10. 浏览器运行时

**这一层要解决的问题：** 用户希望看到网页，模型希望读取和操作网页；如果两者操作两个不同浏览器，登录状态、页面内容和结果就会不一致。

`BrowserRuntime` 抽象统一会话、导航、快照、点击、输入、等待、截图和下载。

### 桌面模式：共享可见页面

Electron 创建每线程浏览器会话对应的 `WebContentsView`。Rust `DesktopBrowserRuntime` 通过带 Token 的回环 broker 操作同一会话，因此用户在 UI 中看到的页面与 Agent 操作的页面是同一个页面。broker 自身由 Electron 主进程拥有，渲染层只能通过白名单 IPC 控制视图布局和导航状态。

### 非桌面模式：CDP 回退

没有可用 Electron broker 时，服务端使用 `LocalBrowserRuntime`，连接本地 Chrome/Edge 的 CDP 运行时。它是功能回退，不会自动得到桌面可见浏览器的会话状态。

### 域名审批和下载

浏览器工具和浏览器面板会检查域名授权。对于需确认的域名，先创建审批记录；用户同意后只写入授权，不自动重试旧请求。下载路径和文件访问仍应通过工作区/预览规则处理，不能把网页下载视为可信输入。

**共享会话的价值：** 用户可以看到模型点击了什么，模型也不会在一个看不见的浏览器里得到与 UI 不同的页面。这提高了可解释性，但也让网页访问成为需要审批和隔离的高风险能力。

---

## 11. 产物、文件预览与 XLSX

**这一层要解决的问题：** 文件预览看似是 UI 功能，实际也涉及路径授权、线程归属和大文件控制。若让浏览器直接按用户传入路径读磁盘，会绕开服务端的安全边界。

### 11.1 预览服务

预览是线程范围内的只读服务，而不是前端直接读取本机文件。请求先解析 `PreviewTarget`：

- 工作区目标必须在当前线程工作区内，且路径规范化后仍是普通文件。
- Artifact 目标必须属于当前线程。
- 服务端按类型和大小限制生成 `PreviewDescriptor`，含预览标识、来源、名称、内容类型、大小和 revision。
- 内容、工作簿元数据和单元格区间分别通过认证的 `/previews/:id/content`、`/workbook`、`/range` 获取。

当前预览种类包括文本、图片、PDF、电子表格和不支持的文件。前端 `PreviewHost` 使用 Monaco 显示只读文本/代码、Blob URL 显示图片、PDF.js 显示 PDF、虚拟化区间网格显示 XLSX。无法预览的格式可以请求系统应用打开，但不会被前端当作脚本执行。

### 11.2 电子表格工具

`spreadsheet` 工具只面向 `.xlsx`：

| action | 作用 | 变更工作区 |
|---|---|---|
| `inspect` | 工作簿摘要 | 否 |
| `list_sheets` | Sheet 列表及属性 | 否 |
| `read_range` | 按零基、包含端点的单元格区间读取 | 否 |
| `write` | 创建新工作簿，或重建源工作簿后写出到指定路径 | 是 |

读取使用 `calamine`，写入使用 `rust_xlsxwriter`。写操作会保留读取到的值、公式、Sheet 顺序和可见性，但不保证复制样式、图表、图片、宏或其他嵌入对象。写入输入支持空值、字符串、整数、数字、布尔和公式；每个请求均校验 Sheet 名称、零基单元格坐标、重复更新、输入/输出大小及总单元格等上限。

表格读写仍先经过 Policy Engine，文件通过 `ExecutionEnvironment` 读写。因此它不是绕过普通工作区权限的专用通道。

**为什么 XLSX 需要专门工具？** XLSX 本质是多个 XML 文件组成的压缩包。让模型直接拼 XML 很容易损坏文件。专用工具把操作提升为“读 Sheet、读范围、写单元格”，并把格式保真等限制明确暴露出来。

---

## 12. React Workbench

**这一层要解决的问题：** 前端要让用户感觉系统是连贯的，但又不能把浏览器内存当成唯一事实来源。它因此同时维护交互状态，并从服务端恢复业务事实。

`apps/desktop/src/App.tsx` 是当前工作台的大部分状态编排层，没有引入 Redux 或 Zustand。它通过 `ApiClient`：

- 初始化平台信息、服务健康、最近工作区、项目/线程、设置、Provider 健康、MCP 和 sandbox 状态；
- 为当前线程加载消息、事件、审批、工作区、产物、上下文、Git 和终端记录；
- 订阅 Agent SSE 与终端 SSE，按序号合并增量状态；
- 打开文件/产物预览标签和可见浏览器标签；
- 将审批、取消、上下文压缩、Git 操作及子智能体操作提交到服务端。

主要组件职责如下：

| 组件 | 职责 |
|---|---|
| `WorkbenchPanel` | 文件、diff、终端、扩展和 sandbox 等工作台页签 |
| `RightContextRail` | 线程上下文、审批、子智能体、Git 操作和运行状态 |
| `XtermTerminal` | xterm.js 的输入、输出和尺寸变化桥接 |
| `ArtifactGallery` | 线程 Artifact 列表和预览入口 |
| `PreviewHost` | 文本、图片、PDF、XLSX 的只读预览 |
| `WebPreviewSurface` / `BrowserPanel` | Electron 共享网页或 CDP 回退页面 |
| `LogViewer` | 通过 preload 读取 Electron 脱敏日志 |

UI 是事件的消费者，但并非“完全不维护状态”：它维护当前选择、加载/错误状态、缓存和 SSE 去重。服务端持久化事件是跨刷新和重启的事实来源。

**这里适合学习“前端状态的两类来源”：** 抽屉是否展开、当前选中哪个标签属于短暂 UI 状态；消息、审批、Turn 和产物属于业务事实。前者可以在刷新后丢失，后者必须从 API/数据库恢复。把两者混在一起是桌面工作台常见的复杂度来源。

---

## 13. 构建、开发与验证

**这一层要解决的问题：** 多语言、多进程项目最容易出现“本机能跑，打包后缺一个二进制或环境变量”的问题。脚本把构建顺序和必要依赖写成可重复执行的步骤。

### 13.1 依赖和入口

- Rust workspace：`opentopia-core`、`opentopia-server`、`opentopia-cli`。
- 桌面 workspace：`apps/desktop`，Vite 构建 React，Electron 负责宿主和打包。
- Windows 开发环境：`scripts/dev-env.ps1` 准备 GNU Rust toolchain、WinLibs、环境别名和默认的本地验证 Token。

### 13.2 服务与桌面启动

```powershell
.\scripts\dev-env.ps1
cargo run -p opentopia-server

pnpm.cmd install
pnpm.cmd dev:desktop
```

直接启动服务端时必须设置符合长度要求的 `OPENTOPIA_API_TOKEN`。`dev-env.ps1` 仅为本地验证提供默认 Token；Electron 启动时会替换为每次启动随机生成的 Token。

### 13.3 打包

`scripts/build-desktop.ps1` 按顺序：

1. 构建 release 版 `opentopia-server`；
2. 将二进制复制到 `apps/desktop/resources/`；
3. 在 Windows 上校验并暂存 Codex restricted-token sandbox helpers；
4. 构建桌面前端并调用 `electron-builder`。

`scripts/check.ps1` 运行 `cargo check --workspace`、桌面 TypeScript typecheck 和前端 build。它是基础静态/构建验证，不替代 Provider、沙箱、浏览器或端到端人工验证。

**验证应分层进行：** 编译检查类型和链接；单元测试检查局部逻辑；服务端验证检查 API；桌面端手工或端到端验证检查真实进程边界。任何一层通过，都不意味着其他层已经正确。

---

## 14. 关键边界与维护原则

1. **所有本地 API 都需要认证。** 新路由必须位于鉴权中间件之后；不能为健康检查或 SSE 建未认证例外。
2. **Electron 不得成为 Rust 安全模型的旁路。** 新 IPC 只能提供桌面能力，文件变更、shell、审批和 sandbox 仍应留在 Rust 服务端。
3. **策略、执行环境与 OS 沙箱要同时检查。** 不要把其中一层的成功当作其他两层已经生效。
4. **事件先持久化，再广播。** 新可恢复 UI 状态需要有 SQLite 事实来源，并支持用序号重放。
5. **审批必须可恢复且动作范围明确。** 允许某个 pending tool call 不应悄悄扩大为整个线程的永久授权。
6. **文件和 Artifact 必须绑定线程/工作区。** 预览、下载、上下文源和 MCP 输入都不能因为前端传入路径而绕过作用域验证。
7. **将计划与已实现能力分开记录。** 对更新签名、远程发布、PR 创建、格式保真 XLSX、远程多用户部署等内容，应在设计或 backlog 中说明，不应写为当前运行时保证。

---

## 15. 一条请求的完整路径

```text
用户在 React 工作台提交消息和可选上下文源
  -> ApiClient 携带 Bearer token 调用本地服务
  -> 服务端校验 Origin/Token、加载源和 Skill 引用、创建并保存 Turn 与用户消息
  -> 后台 AgentCore 流式调用 Provider
  -> 模型按需发出工具调用
  -> PolicyEngine 决定 allow / ask / deny
  -> allow：ExecutionEnvironment 在 sandbox 配置下执行，结果写事件/产物并回传模型
  -> ask：保存 ApprovalContinuation，Turn 等待用户决定
  -> 完成：保存助手消息和 Turn 终态
  -> SQLite 事件经 SSE 历史回放和实时广播返回 UI
  -> UI 根据事件、Turn、审批和产物更新视图；刷新后从 API 恢复状态
```

这条路径体现了 OpenTopia 的核心取舍：UI 负责交互，服务端负责协调和可恢复状态，核心库负责 Agent 能力与安全边界，Electron 只承担桌面特权能力。

---

## 16. 跟读源码的练习路线

下面的路线适合一边读代码、一边验证本文的说法。每一步都只追踪一条因果链，避免一开始陷入所有文件。

### 练习 1：追踪一条用户消息

1. 从 `apps/desktop/src/App.tsx` 中提交消息的处理函数开始，找到 `ApiClient` 的调用。
2. 在 `apps/desktop/src/api/client.ts` 中找到 `POST /api/threads/:thread_id/messages`。
3. 跳到 `crates/opentopia-server/src/main.rs` 的 `send_message`，观察它先创建 Turn、再保存 Message、最后 `tokio::spawn` 后台任务的顺序。
4. 继续跟到 `run_new_agent_turn` 和 `AgentCore::run_turn_detailed_streaming`。

完成后回答：**为什么 HTTP 接口先返回用户消息，而不等待模型最终回答？**

### 练习 2：画出 Turn 状态机

从 `crates/opentopia-core/src/model.rs` 的 `TurnStatus` 开始，画出下面的状态转换：

```text
running -> succeeded
running -> failed
running -> cancelling -> cancelled
running -> waiting_approval
waiting_approval -- 用户允许或拒绝 --> 新建一个 running 的恢复 Turn -> succeeded / failed
进程重启时：running 或 cancelling -> interrupted
```

`waiting_approval` 记录保留为已暂停的执行事实；恢复时创建一个新的 Turn，并从保存的 continuation 继续。再读 `crates/opentopia-server/src/turns.rs`，找出“同一线程不能同时运行两个活动 Turn”是在哪里保证的。这个练习会帮助你理解：状态机不是画图工具，而是用来限制非法状态和恢复异常状态的设计。

### 练习 3：追踪一次工具调用

1. 从 `crates/opentopia-core/src/tools.rs` 的 `ToolRegistry::with_builtins` 选择 `shell` 或 `write_file`。
2. 观察工具如何读取输入 schema，如何调用 `ctx.policy`，以及如何通过 `ctx.environment` 执行。
3. 再读 `policy.rs` 和 `execution.rs`，分别回答“产品规则是否同意？”与“底层如何安全完成？”。
4. 最后读 `sandbox.rs`，理解 OS 级限制为什么仍然需要存在。

完成后回答：**若把策略检查只写在 UI 按钮里，模型、CLI 或未来新 API 能否绕过它？为什么？**

### 练习 4：验证可恢复事件流

1. 在 `main.rs` 中找到 `publish_payload`，确认事件何时写入存储、何时广播。
2. 找到 `stream_events`，观察历史事件和实时 SSE 如何拼接。
3. 回到 `App.tsx`，查看 UI 如何订阅并使用 `seq` 合并事件。

完成后回答：**如果 SSE 在工具执行中断开，用户刷新页面后哪些数据应从 SQLite 恢复，哪些只是短暂 UI 状态？**

### 练习 5：观察一次审批续跑

1. 在 `agent.rs` 中搜索 `ApprovalRequested` 和 `AgentContinuation`。
2. 在 `main.rs` 中阅读 `decide_approval`，比较允许和拒绝的两条路径。
3. 注意 continuation 中保存的是模型对话和待执行调用，而不只是一个布尔值。

完成后回答：**为什么“允许”不能仅仅设置一个全局开关，然后重新发起原始用户消息？**

### 练习 6：自己设计一个新能力

假设要加入一个“读取 JSON 并显示格式化结果”的工具。先不要写代码，先回答：

1. 它应该是 React 组件、HTTP 路由，还是 `ToolRegistry` 中的工具？是否需要三者配合？
2. 输入路径如何验证在工作区中？
3. 它需要 `inspect_read`、`inspect_write` 还是 `inspect_command`？
4. 结果是只返回给模型、保存为 Artifact，还是同时提供预览？
5. 出错、取消、输出过大时各自应该怎样表现？

这五个问题就是架构设计的基本功：先确定边界和失败模式，再决定代码放在哪里。
