# OpenTopia 实现原理（教学版）

> 本文档不讲代码怎么调的，不讲函数之间的调用关系。我们用**大白话 + 生活比喻**来解释 OpenTopia 每个模块"为什么存在"和"怎么工作的"。

---

## 目录

1. [整体架构：餐厅 + 后厨](#1-整体架构餐厅--后厨)
2. [Desktop Shell — 桌面主进程](#2-desktop-shell--桌面主进程)
3. [Platform Bridge — 桌面能力桥](#3-platform-bridge--桌面能力桥)
4. [Workbench UI — 用户界面](#4-workbench-ui--用户界面)
5. [客户端组件库 — 新功能模块](#5-客户端组件库--新功能模块)
6. [Agent Event Protocol — 事件协议](#6-agent-event-protocol--事件协议)
7. [Local Server — 后端服务](#7-local-server--后端服务)
8. [Session Store — 数据持久化](#8-session-store--数据持久化)
9. [Agent Loop — 智能体核心循环](#9-agent-loop--智能体核心循环)
10. [Provider Tool Loop — 模型工具循环](#10-provider-tool-loop--模型工具循环)
11. [Execution Engine — 执行引擎](#11-execution-engine--执行引擎)
12. [Sandbox — 沙箱隔离](#12-sandbox--沙箱隔离)
13. [Permission / Approval — 权限控制](#13-permission--approval--权限控制)
14. [Built-in Tools — 内置工具](#14-built-in-tools--内置工具)
15. [MCP — 模型上下文协议](#15-mcp--模型上下文协议)
16. [Model Provider — 模型接入层](#16-model-provider--模型接入层)
17. [Settings — 设置管理](#17-settings--设置管理)
18. [Workspace — 工作区管理](#18-workspace--工作区管理)
19. [CLI — 命令行入口](#19-cli--命令行入口)
20. [Windows Dev/Build — 开发与构建](#20-windows-devbuild--开发与构建)
21. [总结：一条消息的完整旅程](#21-总结一条消息的完整旅程)
22. [借鉴来源：我们站在谁的肩上](#22-借鉴来源我们站在谁的肩上)

---

## 1. 整体架构：餐厅 + 后厨

OpenTopia 只有**两大部分**：

- **桌面应用（Electron）** = 餐厅本身。包括大堂（用户界面）和餐厅经理（主进程），它们在同一栋楼里，有内部通道连接。
- **Rust 后端** = 后厨（独立厨房）。跟餐厅是分开的，通过点菜窗口（HTTP）沟通。

### 桌面应用内部：经理和大堂

Electron 桌面应用内部又分两个角色，但注意——**它们是同一家店的不同职能，不是独立的楼层**：

- **主进程（Desktop Shell）** = 餐厅经理。顾客看不到他，但他的工作很关键：开店（创建窗口）、关店（退出清理）、检查厨房有没有着火（启动后端）、备好调料（环境变量）、保管保险箱（API Key 安全存储）。经理不直接服务顾客。
- **渲染进程（React UI）** = 整个大堂。包括餐桌（聊天区域）、菜单（操作界面）、工具箱（工作台）——顾客看到的、交互的一切都在这里。
- **Platform Bridge** = 服务员专用的传话本。大堂的服务员不能直接冲进经理办公室，但可以通过这本传话本传递特定的信息，比如"客人想知道厨房地址"、"帮客人开一下大门"、"帮客人选一个工作目录"、"经理帮我存一下这个密码"。

### 桌面应用 ↔ 后厨

桌面应用和后厨（Rust 后端）是**两个独立的进程**，通过 HTTP 通信：

- 大堂写好点菜单（HTTP 请求），从点菜窗口递给后厨
- 后厨收到后喊一声"收到"（HTTP 返回 200），然后开始做菜
- 每做完一道菜就摇铃（SSE 事件推送），服务员听到铃声就去端菜上桌

---

## 2. Desktop Shell — 桌面主进程

**它解决什么问题？**

用户双击图标后，需要一个**完整的桌面应用**——有窗口、有菜单栏、能最小化到系统托盘。浏览器做不到这些，Electron 就是干这个的。

**核心原理：一个会自我修复的管家**

Electron 主进程做的事情比以前想象的多得多：

### 1. 开店关店（窗口管理）

- 开一扇窗户（创建窗口），先让窗户"隐形"，等页面加载好了再"亮出来"，避免白屏闪烁
- 单实例锁定：用户再双击一次，不会开第二个窗口，而是把已有窗口弹到最前面
- 窗口关闭时自动退出应用（macOS 除外，mac 的习惯是不退出）

### 2. 检查厨房有没有火（启动后端）

桌面应用分两部分：前端的网页 + 后端的 Rust 服务器。用户双击后：
1. 先去 `http://localhost:8787/health` 问一下"你活着吗？"（超时 1.2 秒）
2. 如果活着，直接开店
3. 如果没响应，自动启动后端的 exe 文件
4. **每半秒再去问一次**，最多等 15 秒

### 3. 备好调料（环境准备）

给后厨传一堆环境变量：API Key、数据库路径、权限模式、沙箱配置。具体包括：
- 搜索并加载 `.env` 文件（从项目目录或周边项目找）
- API Key 别名映射（兼容旧项目的各种变量名）
- 探测 Codex 沙箱二进制（`codex.exe`），找到后把路径设为 `OPENTOPIA_CODEX_SANDBOX_BIN` 环境变量
- 设置沙箱模式（开发用 best_effort，生产用 enforce）和沙箱工作目录
- Windows 特殊处理：设置 Rust GNU 工具链、把 MinGW gcc 加入 PATH

### 4. 保管保险箱（密钥管理）

API Key 是敏感信息，不能明文保存在文件里。Electron 主进程利用操作系统提供的**安全加密存储**（Windows 上是 DPAPI，macOS 上是 Keychain）来加密保存 API Key：

- 用户通过设置面板输入 API Key → 主进程收到后用 `safeStorage.encryptString` 加密 → 存到 `secrets.json` 文件（加密后的数据）
- 每次启动后端时，主进程解密 API Key 并通过环境变量传给后厨
- 用户看不到解密后的 Key，安全日志里也会自动打码（`[redacted]`）

### 5. 写工作日志（日志系统）

主进程会把启动过程中的重要事件全部记录下来，格式是 JSONL（每行一个 JSON 对象）：
- 启动了哪些进程、花了多长时间
- 后端健康检查的结果
- 未捕获的异常和崩溃（写入专门的 crash 日志）
- 渲染进程崩溃（Electron 的特有事件）

所有的日志内容会自动过滤 API Key、Token 等敏感信息。

### 6. 处理深度链接（Deep Link）

注册 `opentopia://` 协议，让操作系统知道：当用户点击 `opentopia://workspace?path=C:\myproject` 这样的链接时，打开 OpenTopia 并跳转到指定工作区。

### 7. 自动更新（Auto Updater）

非开发模式下，主进程启动后会自动检查更新，发现新版本就下载，下次退出时安装。

### 8. 管理最近工作区

维护一个最近打开的工作区列表（JSON 文件），方便用户快速切换。

---

## 3. Platform Bridge — 桌面能力桥

**它解决什么问题？**

Electron 的安全模型要求网页不能直接操作电脑——否则你打开一个网页，它就能读你硬盘的文件。

但网页有时候确实需要知道一些电脑信息，或者触发一些系统操作。

**核心原理：一扇只开若干条缝的门**

网页（渲染进程）和 Node.js 世界之间有一堵墙。墙上只开了**有限的几条缝**，每条缝都是一个"一问一答"的通道：

| 能力 | 说明 | 为什么需要这条缝 |
|------|------|--|
| `getPlatformInfo` | 获取平台信息（操作系统、后端地址） | 前端需要知道连哪个后端 |
| `openExternal` | 用系统浏览器打开链接 | 用户点外链时 |
| `openPath` | 用系统默认程序打开文件/目录 | 用户想看工作区文件 |
| `selectWorkspace` | 弹出系统文件夹选择对话框 | 让用户选工作目录 |
| `getRecentWorkspaces` | 读取最近工作区列表 | 快速切换 |
| `saveRecentWorkspace` | 保存最近工作区 | 记录用户选择 |
| `listSecretSources` / `setSecret` / `deleteSecret` | 查看和管理加密密钥 | 安全存储 API Key |
| `listLogFiles` / `readLogFile` | 读取日志文件 | 调试和排查问题 |

**为什么只开这几条缝？**

**最小权限原则**——只给恰好够用的能力，不多给。前端永远拿不到真实的 API Key 值，只能问"Key 配好了吗？"（是/否）。

**双重运行模式**

Platform Bridge 被设计成可以在两种模式下工作：
1. **桌面模式**（`window.opentopia` 存在）：通过 Electron IPC 调用系统能力
2. **浏览器模式**（`window.opentopia` 不存在）：用浏览器 API（localStorage）做降级，部分能力不可用

这意味着同一个前端代码可以在 Electron 里跑，也可以在浏览器里跑。

---

## 4. Workbench UI — 用户界面

**它解决什么问题？**

给用户一个**看得见、点得着**的界面：管理多个对话线程、看消息、发指令、审批准入、浏览工作区、看代码差异、操作终端。

**核心原理：三栏布局 + 事件驱动渲染**

- **左侧（Sidebar）**：工作区选择器 + 最近工作区列表 + 线程列表
- **中间（Center Pane）**：聊天主区域（消息列表 + 输入框）
- **右侧（Right Panel）**：多功能工作台面板，包含：

**UI 不「猜」状态，只「消费」事件**

后端每做一步就发一个事件，前端收到什么事件就画什么。收到 `ToolCallStarted` 就显示"工具正在执行"，收到 `ToolCallFinished` 就显示结果。前端不需要自己维护状态机。

**启动流程**

```
加载平台信息 → 检查后端健康 → 获取秘密源信息 → 获取最近工作区
→ 如果后端在线：加载线程列表 + 设置 + 提供者健康 + MCP 服务器
→ 切换到第一个线程：加载消息 + 事件 + 打开 SSE 事件流
→ 同时加载终端历史 + 打开终端 SSE 流 + 初始化 PTY 会话
→ 刷新工作台面板（文件树、差异、沙箱状态、MCP、产物、上下文）
```

**状态集中在 App 组件**

所有核心状态集中在 `App()` 组件，通过 props 向下分发。没有使用 Redux/Zustand 等外部状态管理库。全局状态包括：线程列表、当前消息、事件列表、终端会话、工作区信息、设置等约 30 个状态。

---

## 5. 客户端组件库 — 新功能模块

前端新增了几个独立组件，每一个对应一个桌面 IDE 的典型功能：

### WorkbenchPanel — 多功能工作台

右侧面板是一个**标签页容器**，可以切换：

- **Files（文件）**：浏览工作区文件树，预览文件内容（用 MonacoEditor 显示语法高亮）
- **Diff（差异）**：显示 Git diff，支持逐段 stage/unstage/discard 修改
- **Terminal（终端）**：集成 xterm.js，可以执行命令、看实时输出
- **Extensions（扩展）**：管理 MCP 服务器的启用/禁用
- **Sandbox（沙箱）**：查看沙箱状态和配置

### MonacoEditor — 代码编辑器

基于 Microsoft 的 Monaco Editor（就是 VS Code 用的那个）。支持：
- 语法高亮（通过文件名自动检测语言）
- 只读模式（预览文件时使用）
- Artifact 预览

### XtermTerminal — 终端模拟器

基于 xterm.js + @xterm/xterm，可以在桌面应用内打开一个真正的终端。它：
- 连接到后端的 PTY 会话（伪终端）
- 支持输入（用户在终端里打字）和输出（看到命令结果）
- 支持调整大小（resize）
- 支持关闭会话

### ArtifactGallery — 产物画廊

AI 在运行过程中可能生成"产物"（Artifact），比如生成的代码文件、分析报告等。ArtifactGallery 列出所有产物，点击可以预览。

### LogViewer — 日志查看器

读取和显示 Electron 主进程写入的日志文件（JSONL 格式），方便调试。

---

## 6. Agent Event Protocol — 事件协议

（内容基本不变，核心设计仍然相同）

**它解决什么问题？**

AI 智能体的执行过程不是瞬间完成的——它要思考、调用工具、等结果、再思考。这个过程需要完整记录下来，并且让前端实时看到进展。

**核心原理：不可变的流水日志**

每个事件都有：
- `id`：全局唯一
- `thread_id`：属于哪个对话
- `turn_id`：属于哪个"轮次"
- `seq`：线程内单调递增的序号（由数据库自动分配）
- `created_at`：时间戳
- `payload`：事件内容（13 种变体）

**事件类型**

| 事件 | 含义 |
|------|------|
| `TurnStarted` | 一轮 AI 思考开始 |
| `ModelDelta` | 模型流式输出的文本片段 |
| `ToolCallStarted` | 工具调用开始 |
| `ToolCallFinished` | 工具调用结束（含结果） |
| `AssistantMessage` | 助手完整消息 |
| `FileChanged` | 文件被修改 |
| `ApprovalRequested` | 需要用户审批 |
| `ContextCompacted` | 上下文被压缩 |
| `TokenUsage` | 模型调用的 token 用量统计 |
| `TurnFinished` | 一轮思考正常结束 |
| `TurnSuspended` | Turn 因需要审批而挂起（等待恢复） |
| `TurnCancelled` | Turn 被用户取消 |
| `Error` | 错误 |

**新增事件说明：**
- `TokenUsage`：模型调用完成后发出，包含 `input_tokens`、`output_tokens`、`total_tokens`，供前端展示用量
- `TurnSuspended`：当工具调用触发 `Ask` 审批时，Turn 不再直接结束，而是挂起等待用户决策。事件携带 `approval_id` 和挂起原因，后续可通过 `resume_turn` 恢复
- `TurnCancelled`：用户通过 API 取消正在执行的 Turn 时发出，携带取消原因

**消息和事件的区别？**

- **事件**：底层的执行日志，粒度很细
- **消息**：给用户看的对话单元，粒度较粗

事件是"流水账"，消息是"最终呈现"。两者分离的好处是：你可以改变消息的组装方式而不影响底层日志记录。

---

## 7. Local Server — 后端服务

**它解决什么问题？**

前端需要一个后端接口来发送消息、获取数据。Local Server 就是这个后端，用 Rust 的 Axum 框架写的异步 HTTP 服务。

**核心原理：一个带实时推送的 HTTP API**

### 路由总览

后端提供了约 35 个 API 端点，按功能分组：

| 分组 | 端点 | 作用 |
|------|------|------|
| **健康检查** | `/health` | 问后端活着吗 |
| **设置** | `/api/settings` | 读写设置（模型、权限等） |
| **模型提供者** | `/api/provider/health`、`/api/provider/test` | 查看和测试模型配置 |
| **线程** | `/api/threads` | 创建和列出对话 |
| **消息** | `/api/threads/:id/messages` | 发消息和看消息 |
| **事件** | `/api/threads/:id/events`、`/events/stream` | 拉取和实时推送事件 |
| **Turn 管理** | `/api/threads/:id/turn`、`/turn/cancel` | 查询和取消正在执行的 Turn |
| **终端（命令）** | `/api/threads/:id/terminal/commands` 等 | 执行一次性命令 |
| **终端（会话）** | `/api/threads/:id/terminal/session` 等 | 持久化的 PTY 终端 |
| **工作区** | `/api/threads/:id/workspace/tree` 等 | 浏览文件、看 diff |
| **沙箱** | `/api/threads/:id/sandbox` | 查看沙箱状态 |
| **上下文** | `/api/threads/:id/context` | 上下文预算和压缩 |
| **审批** | `/api/threads/:id/approvals/:id/decision` | 审批决策 |
| **产物** | `/api/threads/:id/artifacts` | 产物管理 |
| **轨迹** | `/api/threads/:id/trajectory` | 导出完整执行轨迹 |
| **MCP** | `/api/mcp/servers` 等 | MCP 服务器管理 |

### SSE 双段拼接：历史 + 实时

前端连接事件流时可以带上 `since=N`，意思是"我已经有 N 号之前的事件了"。

后端的处理方式：
1. 从数据库里查出 `seq > N` 的所有历史事件
2. 同时订阅内存中的广播频道
3. 把历史事件和实时事件拼接成一个流

这样前端断线重连时，一条事件都不会漏。

### 异步执行与 Turn 管理

AI 回复可能要十几秒甚至更久。后端的处理流程现在是：

1. 检查该 thread 是否有待处理的审批请求——有则返回 409 冲突
2. 通过 `TurnManager::begin()` 注册一个新的 Turn，防止同一 thread 的并发执行
3. 保存用户消息到数据库，返回 200
4. 用 `tokio::spawn` 启动后台任务

后台任务通过 `mpsc::UnboundedChannel` 实时接收 Agent 发出的事件，每收到一个就立即持久化到 SQLite 并通过 EventBus 广播给 SSE 订阅者。如果用户取消 Turn，`CancellationToken` 会传播到执行引擎，终止正在运行的子进程。

### API 认证系统

所有 `/api/*` 路由现在都经过认证中间件保护。后端启动时要求设置 `OPENTOPIA_API_TOKEN` 环境变量（至少 32 字节）。前端每次请求需要在 HTTP 头中携带 `Authorization: Bearer <token>`。

认证架构分为两层：

**第一层：CORS 源检查**——浏览器请求必须在允许的源列表中：
- `null`（Electron 内嵌页面的 origin）
- `file://`（本地文件加载）
- `http://127.0.0.1:5173`（Vite 开发服务器）
- `http://localhost:5173`
- 可通过 `OPENTOPIA_DEV_ORIGIN` 环境变量添加额外开发源（必须是 loopback 地址）

**第二层：Bearer Token 校验**——使用常数时间比较（`constant_time_eq`）防止时序攻击。验证失败时返回 `401 Unauthorized`。

注意：Electron 主进程在启动后端时，通过 `createBackendEnv` 自动设置 `OPENTOPIA_API_TOKEN`，preload 脚本中的 fetch 请求自动添加 Authorization 头，用户不需要手动配置。浏览器模式下仍需手动配置。

### 多提供者支持

后端现在支持管理**多个模型提供者**，用户可以在设置面板添加多个 provider（比如一个用 GPT-4，一个用本地模型），在它们之间切换，测试连接是否正常。

---

## 8. Session Store — 数据持久化

**它解决什么问题？**

用户关闭应用再打开，聊天记录不能丢。所有数据必须持久化到硬盘。

**核心原理：一个本地的 SQLite 档案馆**

SQLite 是一种嵌入式的数据库，不需要安装服务器。OpenTopia 用多个表来存数据：

| 表 | 相当于 | 存什么 |
|------|--------|--------|
| `threads` | 档案盒 | 对话标题、工作区路径 |
| `messages` | 文件 | 谁说了什么、包含哪些内容 |
| `events` | 执行日志 | 每一步操作的详细记录 |
| `approvals` | 审批单 | 审批请求和决策 |
| `artifacts` | 产物清单 | AI 生成的文件或内容 |
| `terminal_history` | 终端日志 | 执行过的命令和输出 |
| `mcp_servers` | 扩展配置 | MCP 服务器设置 |
| `thread_mcp_servers` | 扩展绑定 | 线程与 MCP 的关联 |
| `settings` | 配置单 | 应用设置 |

**为什么用 Trait 抽象？**

代码里定义了 `SessionStore` trait（接口），包含线程、消息、事件、终端历史、产物、审批的核心操作。目前只有 `SqliteSessionStore` 一种实现。

但 `SqliteSessionStore` 还额外实现了 trait 范围之外的方法——设置管理（`load_settings`、`save_settings`）和 MCP 服务器管理（CRUD 操作）。这些方法不在 trait 中，是因为它们属于存储层的具体实现细节，未来如果换成 PostgreSQL，"设置"和"MCP 配置"的存取方式可能完全不同。

这就是面向接口编程：不依赖具体实现，依赖抽象约定。需要扩展时只加 trait 方法，不改调用方。

---

## 9. Agent Loop — 智能体核心循环

**它解决什么问题？**

用户在输入框里说"帮我看看项目结构"，AI 要理解这句话、决定怎么处理、调用工具、组织回复。

**核心原理：两条路径 + MCP 集成 + 流式执行 + 挂起/恢复**

Agent 收到用户消息后，先做一道选择题：

### 路径一：命令直达（不走 AI）

如果用户输入的是 `/list`、`/read`、`/write`、`/run`、`/diff`、`/patch`、`/mcp` 这类**确定性命令**，Agent 直接执行对应的工具函数，不经过 AI。好处是极速响应。

### 路径二：LLM 推理（走 AI + 工具循环）

如果用户说的是自然语言，Agent 做了两件事：
1. **先自动做一次 `list_files`**：把工作区根目录的文件列表发给 AI，让 AI 知道项目结构
2. **把控制权交给 AI + 工具循环**：AI 可以自主决定调用哪些工具、按什么顺序调用

### Turn 的新模型：Completed / Suspended

以前的 Turn 只有"完成"一个结局。现在 Turn 可以有三种结局：

- **Completed**：正常完成，输出最终回复，发出 `TurnFinished` 事件
- **Suspended**：工具调用触发 `Ask` 审批决策 → 发出 `ApprovalRequested` + `TurnSuspended` → Turn 挂起等待用户决策。后端将 Turn 的完整上下文（`AgentContinuation`）持久化到数据库，后续可通过 `resume_turn_streaming` 恢复执行
- **Cancelled**：用户通过 API 取消 → 发出 `TurnCancelled` 事件 → 正在执行的子进程被 `CancellationToken` 终止

### 流式执行与实时事件推送

Agent 执行过程中产生的所有事件，不再等 Turn 结束后才发，而是通过 `mpsc::UnboundedChannel` 实时推送到主循环，主循环一边接收事件一边通过 SSE 推给前端：

```
Agent.run_turn_detailed_streaming(input, sink)
  → 把 AgentEventSender 注入 TurnEvents
  → 每产生一个事件就 sender.send(payload)
  → 主循环 receiver.recv() 收到后立即持久化 + SSE 发布
```

这意味着前端可以实时看到 `ModelDelta`（模型逐字输出）、`ToolCallStarted`、`ToolCallFinished`，不需要等整个 Turn 完成。

### 取消机制（Cancellation）

每个 Turn 都有一个 `CancellationToken`。Agent 在创建 `ToolContext` 时注入这个 token，`execution.rs` 中的 `LocalExecutionEnvironment.exec()` 使用 `tokio::select!` 监听取消信号。用户通过 `POST /api/threads/:id/turn/cancel` 触发取消时：
1. `TurnManager` 标记取消状态
2. `CancellationToken` 触发
3. 正在执行的子进程被 kill
4. 发出 `TurnCancelled` 事件
5. Turn 结束

### 并发防护（TurnManager）

每个线程（thread）同一时间只能有一个正在执行的 Turn。`send_message` 和 `decide_approval` 在启动新 Turn 前先通过 `TurnManager::begin()` 检查——如果该 thread 已有运行中的 Turn，则拒绝并返回 409 冲突。Turn 完成后调用 `TurnManager::finish()` 释放锁。

### Turn 状态查询

前端可以通过 `GET /api/threads/:id/turn` 查询当前 Turn 的状态：`{ turn_id, status: "running" | "cancelling", started_at }`。

新增的命令：
- `/search path -- query`：在文件中搜索内容
- `/mcp server__tool {"arg":"value"}`：调用 MCP 工具

### 路径二：LLM 推理（走 AI + 工具循环）

如果用户说的是自然语言，Agent 做了两件事：

1. **先自动做一次 `list_files`**：把工作区根目录的文件列表发给 AI，让 AI 知道项目结构
2. **把控制权交给 AI + 工具循环**：AI 可以自主决定调用哪些工具、按什么顺序调用

### Turn 是什么？

"Turn" 是**一次完整的请求-响应循环**：
1. 用户发一条消息 → 触发一个 Turn
2. Turn 开始 → 产生一系列事件
3. Turn 结束 → 输出最终回复

一个 Turn 产生的所有事件共享一个 `turn_id`，方便追溯。

### MCP 工具同步

Agent 在每次执行前，会从 MCP Host 拉取所有已注册的 MCP 工具，把它们同步到工具注册表中。这样 AI 就可以像调用内置工具一样调用 MCP 工具。

### 上下文预算

当前 `GET /context` 根据消息内容估算 token 使用量并返回最近一次 durable
summary。它还没有 provider-reported 精确 token accounting，也不会按阈值自动压缩。

用户触发压缩后，服务端把有界的 messages + typed events 轨迹发给当前
OpenAI-compatible provider，要求保留目标、决策、文件路径、命令、验证结果和未解决
问题。生成结果以 `ContextCompacted` 事件持久化，metadata 记录 provider、model、
mode 和 covered sequence；后续普通 turn 以及审批继续执行都会把最新 summary 放入
模型请求。手工 summary 仍作为显式覆盖入口保留。

---

## 10. Provider Tool Loop — 模型工具循环

**它解决什么问题？**

AI 不只是一个"一问一答"的聊天机器人，它可以**主动决定调用工具**来获取信息或执行操作。而且它可以连续调用多个工具，每一步都根据前一步的结果决定下一步做什么。

**核心原理：最多 8 轮的对话式执行**

```
第 0 步：给 AI 发消息 + 工具列表
  ↓
AI 回复 → 包含工具调用请求（比如 "帮我读 package.json"）
  ↓
Agent 执行工具 → 把结果发给 AI
  ↓
AI 再回复 → 可能再要调用工具（"再读一下 src/index.ts"）
  ↓
Agent 再执行 → 再发给 AI
  ↓
...
  ↓
AI 回复纯文本 → 完成
```

**关键设计**：
- 最多 8 轮工具调用，防止无限循环
- 每轮 Agent 都把之前的工具调用历史发给 AI，让 AI 知道上下文
- 达到上限后，如果 AI 还想调用工具，Agent 会收集已完成的工具结果，不再继续

### 审批如何处理？（挂起/恢复模型）

当某步工具执行触发了权限系统的 `Ask` 决策（危险操作需要用户确认），Agent 现在的行为是：
1. 发出 `ApprovalRequested` 事件（前端弹审批卡）
2. 发出 `TurnSuspended` 事件（标记 Turn 被挂起）
3. 将当前 Turn 的完整上下文打包成 `AgentContinuation`，持久化到数据库的 `approvals` 表（`continuation_json` 字段）
4. Agent 返回 `AgentTurnOutcome::Suspended`，Turn 执行线程完成

用户在界面上点"允许"或"拒绝"时：
1. `POST /api/threads/:id/approvals/:id/decision` 被调用
2. 后端从数据库读取 `AgentContinuation`
3. 启动一个新的后台任务，调用 `agent.resume_turn_streaming(continuation, approved)`
4. 如果允许：以 `FullAccess` 权限继续执行被暂停的工具，然后回到 Provider Tool Loop
5. 如果拒绝：向模型返回一条"用户拒绝了此调用"的工具结果（`approvalDenied: true`），让模型决定下一步

相比旧版"以 FullAccess 重新执行整个 Turn"的方式，新版只在暂停点精确恢复，不重复执行已经完成的操作。

---

## 11. Execution Engine — 执行引擎

**它解决什么问题？**

Agent 需要执行命令、读写文件，但这些操作需要统一的安全控制和资源管理。

**核心原理：一个带沙箱的进程管理器**

执行引擎是一个 trait（接口），定义了这些操作：
- `exec`：执行一条命令，等待返回（适合一次性命令）
- `spawn_stdio`：启动一个交互式进程，可以持续读写 stdin/stdout/stderr
- `read_file`：读取文件
- `write_file`：写入文件
- `apply_patch`：应用 git patch
- `cancel`：取消正在执行的命令

### 本地执行环境

默认实现 `LocalExecutionEnvironment` 是真正干活的地方：

1. **路径解析**：相对路径自动拼接到工作区根目录
2. **沙箱集成**：执行命令前通过 `build_local_sandbox_command` 检查是否需要沙箱包装
3. **资源限制**：支持设置超时时间、输出大小上限
4. **并发控制**：通过 `CancellationToken` 支持随时取消正在执行的命令
5. **输出截断**：超出上限的输出自动截断，加上 `[output truncated by resource limit]` 标记

### 执行过程（exec 方法）

```
接收 ExecRequest（程序、参数、工作目录、stdin）
→ 通过沙箱构建命令计划（可能包装 bwrap/sandbox-exec）
→ spawn 子进程
→ 注册 CancellationToken（支持通过 request_id 取消）
→ 异步读取 stdout/stderr（超出限制则自动截断）
→ 等待结果（超时/手动取消/输出超限/正常退出）
→ 返回 ExecResult（stdout, stderr, exit_code, success, truncated）
```

### Stdio 交互式会话（spawn_stdio）

跟 exec 不同，spawn_stdio 不是等命令执行完再返回，而是返回一个 `StdioSession` 对象，调用者可以：
- 持续写入 stdin
- 持续读取 stdout/stderr
- 最后调用 `close` 等待命令结束
- 随时调用 `kill` 强制终止

---

## 12. Sandbox — 沙箱隔离

**它解决什么问题？**

AI 可以执行任意 Shell 命令。如果不加限制，一个 `rm -rf /` 或者恶意脚本可能会造成严重破坏。沙箱就是给命令执行加上**操作系统级别的隔离**。

**核心原理：三种模式 + 三个平台**

### 三种运行模式

| 模式 | 行为 | 适用场景 |
|------|------|----------|
| Disabled | 不隔离，直接执行 | 开发调试 |
| BestEffort | 能找到沙箱就用，找不到就显式报告直通 | 开发默认 |
| Enforce | 没有沙箱就报错，绝不直通 | 打包应用默认 |

### 三个平台的实现

OpenTopia 不在应用层做沙箱，而是利用各操作系统原生的沙箱机制：

| 平台 | 沙箱工具 | 原理 |
|------|----------|------|
| Linux | bubblewrap（bwrap） | 用 Linux namespace 隔离文件系统、网络、进程 |
| macOS | sandbox-exec（Seatbelt） | 用 macOS 的 Seatbelt 强制访问控制 |
| Windows | codex restricted-token | 用 Windows 受限令牌 + 桌面隔离 |

### Linux 沙箱（bubblewrap）

bubblewrap 是 Linux 上最常用的沙箱工具。OpenTopia 用它：
- 创建独立的 PID/IPC/UTS namespace
- 只读挂载系统目录（`/bin`, `/etc`, `/usr`, `/lib` 等）
- 读写挂载工作区目录
- 如果配置了网络限制，加上 `--unshare-net`

### macOS 沙箱（sandbox-exec）

macOS 有内置的 Seatbelt 沙箱。OpenTopia 生成一个 Seatbelt 配置文件，内容大致是：
- 默认拒绝所有操作
- 允许读取系统路径
- 允许读写工作区路径
- 根据网络策略决定是否允许网络访问

### Windows 沙箱（codex restricted-token）

Windows 不自行重写高风险的 token/ACL/job-object 代码，而是调用 Codex 的
`codex sandbox` CLI 边界。Electron 主进程启动时会自动搜索 `codex.exe`
（依次检查 `OPENTOPIA_CODEX_SANDBOX_BIN` 环境变量、打包目录下的
`resources/codex-sandbox/codex.exe`、`~/.codex/plugins/.plugin-appserver/codex.exe`），
找到后通过 `CODEX_HOME` 和 `OPENTOPIA_SANDBOX_WORKSPACE` 环境变量将路径传递给
Rust 后端。安装包包含 `codex.exe`、command runner、sandbox setup helper 和
Apache-2.0 license。严格模式下 helper 缺失会失败关闭；本仓库测试会实际尝试写入
工作区外的非临时路径，并断言 restricted-token 后端拒绝该操作。

Linux/macOS adapter 已生成严格的 bubblewrap/Seatbelt 策略并在后端缺失时失败，
但仍需在各自原生发布机跑端到端 confinement 测试；不能把 Windows 验证结果外推成
三平台都已完成生产认证。

### 沙箱状态查询

前端可以通过 API 查询当前沙箱的状态，了解：
- 沙箱类型（local/docker/remote）
- 生命周期状态（ready/starting/stopped/error）
- 隔离模式（disabled/best_effort/enforce）
- 网络策略（inherit/allow/deny）
- 启用状态和可用性

---

## 13. Permission / Approval — 权限控制

**它解决什么问题？**

AI 智能体可以执行命令、读写文件。如果不加限制，它可能误删文件、读到敏感信息。

**核心原理：五级权限 + 三种决策 + 命令规则**

（五级权限表和之前一样，这里不再重复）

**新增：命令策略规则**

除了按模式判断，现在还可以设置**精细的命令匹配规则**：

```rust
CommandPolicyRule {
    pattern: "npm install",      // 匹配模式
    match_kind: Prefix,          // 匹配方式（前缀 / 包含）
    effect: Allow,               // 策略效果（Allow / Ask / Deny）
    reason: "包安装是安全的",     // 原因说明
}
```

**新增：MCP 工具权限**

MCP 工具也有权限标签。工具声明可以带注解（annotations）：
- `readOnlyHint: true` → 只读工具，安全
- `destructiveHint: true` → 破坏性工具，需要谨慎
- `openWorldHint: true` → 涉及网络访问

这些标签映射为权限标签，通过 `ToolPermissionDescriptor` 传给策略引擎判断。

**双重防护（文件路径检查）**

1. 第一层：拒绝任何包含 `..`（父目录）的路径
2. 第二层：把路径解析成绝对路径后，检查是否在工作区内

---

## 14. Built-in Tools — 内置工具

（结构基本一致，但新增了搜索工具和 MCP 工具包装器）

| 工具 | 相当于 | 做什么 |
|------|--------|--------|
| list_files | 侦察兵 | 查看目录下有什么文件 |
| read_file | 阅读者 | 读取文件内容（限制 16000 字符） |
| write_file | 文书 | 写入/修改文件 |
| shell | 万能工 | 执行命令（默认 30 秒超时） |
| git_diff | 质检员 | 查看代码变更 |
| apply_patch | 修理工 | 应用代码补丁 |
| search | 侦探 | 先用 ripgrep 搜索，找不到回退到逐行子串扫描（限制 4096 字符） |
| mcp__* | 外援 | 由 MCP 插件提供的工具（动态注册） |

**工具执行上下文**

每个工具执行时都会带一个 `ToolContext`，包含：
- `workspace_root`：工作区根目录
- `policy`：策略引擎（用于权限判断）
- `environment`：执行环境（带沙箱，封装了 exec/read_file/write_file 等能力）
- `store`：数据存储（可选，用于记录产物）
- `thread_id`：当前线程 ID
- `cancel`：可选的 `CancellationToken`，用于支持工具执行被用户取消

---

## 15. MCP — 模型上下文协议

**它解决什么问题？**

OpenTopia 的工具是有限的（list_files、read_file 等 7 个）。但世界上有成千上万有用的工具——数据库查询、天气查询、代码分析等等。MCP（Model Context Protocol）就是**一个开放的工具接入标准**，让任何人写的外挂工具都能被 OpenTopia 使用。

**核心原理：JSON-RPC over stdio**

MCP 的标准实现方式是：MCP 服务器是一个独立的子进程，OpenTopia 通过 stdin/stdout 与它通信，通信协议是 JSON-RPC 2.0：

```
OpenTopia → stdin: {"jsonrpc":"2.0","id":1,"method":"tools/list"}
MCP 服务器 ← stdout: {"jsonrpc":"2.0","id":1,"result":{"tools":[...]}}

OpenTopia → stdin: {"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"echo","arguments":{"text":"hello"}}}
MCP 服务器 ← stdout: {"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"echo: hello"}]}}
```

可以理解成一个"动手能力极强的外援"——你跟他说一声，他就自己跑起来，通过 stdin/stdout 跟你对话。

### 架构

```
McpExtensionHost（宿主）
  ├── 管理多个 McpServer
  │   ├── McpServer A（例如：文件系统服务器）
  │   │   ├── 工具: read, write, search
  │   │   └── 状态: ready / error / disabled
  │   ├── McpServer B（例如：数据库服务器）
  │   │   ├── 工具: query, schema
  │   │   └── 状态: ready / error / disabled
  │   └── ...
  └── 全局工具路由表（public_name → server_id + tool_name）
```

### 生命周期

1. **注册**：用户在设置面板添加 MCP 服务器（输入命令、参数、环境变量等）
2. **启动**：OpenTopia spawn 子进程，通过 initialize 握手，确认协议版本
3. **发现**：调用 `tools/list`，获取服务器提供的所有工具列表
4. **路由**：每个工具被赋予一个全局唯一的名字（`服务器名__工具名`），注册到路由表
5. **调用**：通过路由表找到对应的服务器和工具名，调用 `tools/call`
6. **停止**：关闭 stdin，杀掉子进程

### 工具名防止冲突

MCP 工具名可能跟内置工具名冲突。OpenTopia 的处理方式：
- 内置工具：`list_files`, `read_file`, `write_file`, `shell`, `git_diff`, `apply_patch`, `search`
- MCP 工具：`服务器名__工具名`（例如 `file_system__read_file`）

这样即使 MCP 服务器也有个叫 `read_file` 的工具，也不会冲突。

### 安全性

MCP 工具调用也会经过权限系统检查。工具声明中的注解（annotations）会映射为权限标签，策略引擎根据这些标签将工具分为五类风险等级并决定是否放行：

| 注解标记 | 映射标签 | 风险分类 | 行为（以 Auto 模式为例） |
|---------|---------|---------|------------------------|
| `readOnlyHint: true` | `read` | 只读 | Allow（直接放行） |
| 无注解或仅有 write | — | 写入 | 同文件写入权限规则 |
| `destructiveHint: true` | `destructive` | 破坏性 | Ask（需审批） |
| `openWorldHint: true` | `network` | 网络 | Ask（需审批） |
| `permissionLabels: ["secret"]` | `secret` | 秘密 | Ask（需审批） |
| 以上均无 | `unknown` | 未知 | Ask（需审批）

---

## 16. Model Provider — 模型接入层

**它解决什么问题？**

OpenTopia 需要调用大语言模型（LLM），但 LLM 有很多种。Model Provider 就是统一接入层。

**核心原理：一个通用的模型插头 + 流式输出**

```rust
trait ModelProvider: Send + Sync {
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse>;
    async fn stream(
        &self,
        request: ModelRequest,
        on_delta: &mut dyn FnMut(ModelStreamDelta) -> Result<()>,
    ) -> Result<ModelResponse>;
    async fn check_health() -> Result<ProviderHealthCheck>;
}
```

`stream` 方法是新增的。与 `complete` 不同，它不是等全部结果返回，而是通过回调函数 `on_delta` 逐步推送三种类型的增量：

- `ModelStreamDelta::Text { text }`：模型输出的文本片段（逐片段推送，前端可实时显示）
- `ModelStreamDelta::ToolCall { index, id, name, arguments_delta }`：工具调用声明
- `ModelStreamDelta::Usage { usage }`：完成后的 token 用量统计

Agent Loop 调用 `stream` 而非 `complete`，产生的 Text delta 实时转换为 `ModelDelta` 事件通过 SSE 推给前端，让用户看到 AI "逐字输出"的效果。

`ModelRequest` 现在不只是简单的文本，还可以包含：
- `system_prompt`：系统提示词
- `conversation`：多轮对话历史（`Vec<ModelConversationMessage>`），结构化表示之前的对话记录
- `user_message`：用户消息
- `tool_candidates`：可以调用的工具列表（AI 可以自主选择调用）
- `previous_tool_calls`：之前已经调用的工具
- `tool_results`：之前工具调用的结果

`ModelResponse` 现在也包含：
- `text`：AI 的文本回复
- `tool_calls`：AI 要求调用的工具列表
- `usage`：可选的 `ModelUsage`，包含 `input_tokens`、`output_tokens`、`total_tokens`、`cached_input_tokens`、`reasoning_tokens`

### 三种 API 格式兼容

`OpenAiCompatibleProvider` 能同时处理三种不同的 API 响应格式：

1. **Chat Completions API**（标准）：从 `choices[0].message.content` 取文本，从 `choices[0].message.tool_calls` 取工具调用
2. **Legacy function_call**（旧版兼容）：从 `choices[0].message.function_call` 取函数调用
3. **Responses API**（新版）：从 `output` 数组中取 type 为 `function_call` 的条目

### 健康检查策略

健康检查分两步：先用 `GET /models`（超时 5 秒），如果失败则用最小请求 `POST /chat/completions` 测试连通性。检查结果包含 `reachable`、`latency_ms`、`model_available`、`error`。

现在可以配置**多个模型提供者**：

| 字段 | 说明 |
|------|------|
| `id` | 提供者 ID（如 "default"、"my-local-model"） |
| `kind` | 类型（Mock / OpenAiCompatible） |
| `base_url` | API 地址 |
| `model` | 模型名称 |
| `api_key_source` | API Key 来源（环境变量名） |
| `api_key_configured` | 是否已配置 Key |

用户在设置面板可以：
- 添加/删除提供者
- 切换当前使用的提供者
- 修改提供者的 base URL 和 model
- 测试连接是否正常

### 健康检查

每个提供者支持健康检查，检查结果包含：
- `reachable`：API 地址是否可达
- `latency_ms`：延迟
- `model_available`：模型是否可用
- `error`：错误信息（如果有）

---

## 17. Settings — 设置管理

**它解决什么问题？**

用户需要配置模型、权限模式、工作区等。这些设置需要持久化。

**核心原理：一个持久化的配置对象**

```rust
AppSettings {
    providers: Vec<ProviderSettings>,   // 多个模型提供者
    active_provider_id: String,         // 当前使用的提供者
    permission_mode: PermissionMode,    // 权限模式
    default_workspace_root: Option<PathBuf>,  // 默认工作区
    updated_at: DateTime<Utc>,          // 最后修改时间
}
```

**多提供者如何切换？**

用户在前端设置面板：
1. 选择一个提供者作为 active
2. 修改它的 base URL / model / apiKeySource
3. 点击保存 → PATCH 到后端
4. 后端更新设置 → 重新创建 AgentCore（使用新的提供者配置）

**从环境变量初始化**

启动时如果没有持久化的设置，会从环境变量自动推断：
- `OPENTOPIA_OPENAI_BASE_URL` → base URL
- `OPENTOPIA_MODEL` → model
- `OPENTOPIA_API_KEY` → API Key
- `OPENTOPIA_PERMISSION` → 权限模式

---

## 18. Workspace — 工作区管理

**它解决什么问题？**

AI 需要跟用户的工作目录打交道——看目录结构、读文件内容、查看 Git 变更、修改文件。

**核心原理：一组只读的工作区查看 API**

工作区 API 都是**只读的**（实际写文件通过工具系统，受权限控制）：

| API | 作用 |
|-----|------|
| `workspace/tree` | 列出目录内容（只显示一层） |
| `workspace/file` | 读取文件内容（限制 64000 字符） |
| `workspace/diff` | 查看 Git diff |
| `workspace/diff/revert` | 安全地恢复文件修改 |
| `workspace/diff/hunk` | 逐段 stage/unstage/discard |
| `trajectory` | 导出完整执行轨迹（含消息、事件、审批、产物、diff） |

### 安全性

所有路径操作都做了安全检查：
1. 拒绝包含 `..` 的路径
2. 路径必须是工作区的子路径
3. 不存在的路径返回 404

### Git Diff 操作

后端的 git diff 功能支持：
- 查看暂存区（staged）和未暂存区（unstaged）的差异
- 解析 diff hunk（逐段修改块）
- 安全地应用 / 撤销 hunk

---

## 19. CLI — 命令行入口

（基本不变，但值得一提）

**核心原理：复用一切，只是入口不同**

CLI 和桌面后端共享同一套 Rust 代码——`AgentCore`、`SqliteSessionStore`、工具集全都一样。

CLI 命令：
- `opentopia threads` — 列出所有对话
- `opentopia new --title "..."` — 创建新对话
- `opentopia send <id> "<msg>"` — 发消息

**设计原则**：CLI 不做任何与桌面端不同的逻辑。你在桌面端创建的对话，在 CLI 里也能看到。

---

## 20. Windows Dev/Build — 开发与构建

（基本覆盖之前的版本，新增要点）

**沙箱二进制分发**

构建时会检测 `codex-sandbox` 二进制（用于 Windows 沙箱），将其路径通过环境变量传给后端。

**打包流程**

```
1. 导入 dev-env.ps1（确保环境正确）
2. cargo build --release -p opentopia-server
3. 复制二进制到 apps/desktop/resources/
4. pnpm --filter @opentopia/desktop dist
5. 检测 codex-sandbox 路径并注入环境变量
```

---

## 21. 总结：一条消息的完整旅程

把整个流程串起来讲一遍：

> 用户在输入框打字 → 前端 POST 给后端 → 后端存消息并启动 Agent → Agent 自动 list_files → Agent 把文件列表 + 用户消息发给 LLM → LLM 决定调用工具 → Agent 检查权限 → 通过沙箱执行工具 → 结果发给 LLM → （循环最多 8 轮） → LLM 输出最终回复 → 存为 AssistantMessage → 所有事件通过 SSE 推回前端 → 前端实时渲染

或者用比喻：

1. 你（顾客）跟服务员（前端 UI）说："帮我看看项目结构，然后优化构建脚本"
2. 服务员写张条子（HTTP 请求）递给后厨（Rust 后端）
3. 后厨喊了一声"收到"，然后开始忙活（异步 Agent）
4. 后厨先看了一眼冰箱（`list_files`）
5. 然后把冰箱里的东西和你的要求一起告诉大厨（LLM）
6. 大厨说："先看看 package.json"（ToolCall）
7. 后厨检查了权限，打开冰箱拿出 package.json 念给大厨听
8. 大厨听完说："改这几行"（另一个 ToolCall）
9. 后厨改了文件，发出 `FileChanged` 事件
10. 大厨满意了，说"搞定了"（AssistantMessage）
11. 后厨每做一步就摇铃（SSE 事件），服务员听到铃声就端菜上桌
12. 如果大厨要用你的私房调料（危险操作），后厨会喊"顾客同意吗？"（ApprovalRequested）

整个过程中，后厨的每一项操作都写在流水账里（SQLite 持久化），端上桌的每一道菜都有视频回放（事件轨迹导出）。

---

## 22. 借鉴来源：我们站在谁的肩上

OpenTopia 没有从零发明一切。每个模块的设计都参考了业内已有的开源项目——不是复制代码，而是**理解其架构模式后重新实现**。

以下是各模块的借鉴来源和具体对应关系：

### 桌面壳（Desktop Shell）

| 来源项目 | 借鉴了什么 | 落到 OpenTopia 哪里 |
|----------|-----------|-------------------|
| **Goose** | Electron 主进程架构、窗口生命周期、设置/菜单/更新的主进程承载 | `apps/desktop/electron/main.cjs` |
| **opencode** | sidecar 后端健康检查、自动拉起、进程清理 | `startBackendIfNeeded()` 轮询策略 |

### 桌面能力桥（Platform Bridge）

| 来源项目 | 借鉴了什么 | 落到 OpenTopia 哪里 |
|----------|-----------|-------------------|
| **opencode** | `PlatformProvider` 风格的桌面能力抽象——平台信息、路径操作、对话框 | `apps/desktop/electron/preload.cjs` 的 `contextBridge` 和 `apps/desktop/src/platform.ts` 的降级兼容层 |

### 用户界面（Workbench UI）

| 来源项目 | 借鉴了什么 | 落到 OpenTopia 哪里 |
|----------|-----------|-------------------|
| **Codex / Trae** | 三栏工作台布局（左侧线程、中间聊天、右侧工作区） | `apps/desktop/src/App.tsx` 的 layout 结构 |
| **opencode** | 可换主题、token 化 UI、工作台视觉基调 | `apps/desktop/src/styles/app.css` 的样式体系 |

### Agent 事件协议（Agent Event Protocol）

| 来源项目 | 借鉴了什么 | 落到 OpenTopia 哪里 |
|----------|-----------|-------------------|
| **Codex** | SQ/EQ 异步事件队列、类型化事件载荷（typed event payload） | `crates/opentopia-core/src/model.rs` 的 `AgentEventPayload` 枚举 |
| **OpenHands** | event trajectory 思路——完整记录每一步操作，支持回溯和回放 | `AgentEvent` 的 `seq` 序号和 `turn_id` 归组 |

### 后端服务（Local Server）

| 来源项目 | 借鉴了什么 | 落到 OpenTopia 哪里 |
|----------|-----------|-------------------|
| **Codex** | REST + SSE 事件流、审批决策 API 模式 | `crates/opentopia-server/src/main.rs` 的异步 agent 执行和 SSE 双段拼接 |
| **Codex** | Turn 管理——运行中 Turn 的并发控制和取消 | `crates/opentopia-server/src/turns.rs` 的 `TurnManager` |
| **Codex** | Bearer token API 认证与 CORS 安全 | `crates/opentopia-server/src/auth.rs` 的 `ApiAuth` |
| **OpenHands** | event service 抽象、分页/增量查询 | `since` 参数的事件列表和持久化 |
| **Goose** | provider/tool/session 组合的服务端编排 | `AppState` 中的 agent + settings + mcp_host 组合 |

### 数据持久化（Session Store）

| 来源项目 | 借鉴了什么 | 落到 OpenTopia 哪里 |
|----------|-----------|-------------------|
| **OpenHands** | conversation + event 持久化模式、SQLite schema 设计 | `crates/opentopia-core/src/store.rs` 的 `SqliteSessionStore` |

### 智能体循环（Agent Loop）

| 来源项目 | 借鉴了什么 | 落到 OpenTopia 哪里 |
|----------|-----------|-------------------|
| **Goose** | agent loop 结构——provider 调用 + 工具执行 + session 管理 | `crates/opentopia-core/src/agent.rs` 的 `AgentCore` 和 `run_turn` |
| **Goose** | 确定性本地工具命令（/list, /read 等）不走 LLM 直接执行 | `ParsedTask::parse` 命令解析器 |
| **Codex** | MCP 工具作为 Agent 能力扩展层 | `sync_mcp_tools()` 和 `McpToolWrapper` |

### 执行引擎（Execution Engine）

| 来源项目 | 借鉴了什么 | 落到 OpenTopia 哪里 |
|----------|-----------|-------------------|
| **Codex** | 带沙箱的进程执行、输出截断、资源限制 | `crates/opentopia-core/src/execution.rs` 的 `LocalExecutionEnvironment` |
| **portable-pty crate** | 跨平台伪终端（PTY）支持 | `PtySession` 和后端的交互式终端 |

### 沙箱隔离（Sandbox）

| 来源项目 | 借鉴了什么 | 落到 OpenTopia 哪里 |
|----------|-----------|-------------------|
| **Codex** | OS 级沙箱架构：Linux bwrap + seccomp、macOS Seatbelt、Windows restricted token | `crates/opentopia-core/src/sandbox.rs` 的三平台实现 |
| **Codex** | 沙箱命令包装策略——在原始命令外套一层沙箱工具 | `build_local_sandbox_command` 的 `SandboxCommandPlan` |

### 权限控制（Permission / Approval）

| 来源项目 | 借鉴了什么 | 落到 OpenTopia 哪里 |
|----------|-----------|-------------------|
| **Codex** | 命令风险判定、exec policy、patch 工具进入权限链路 | `crates/opentopia-core/src/policy.rs` 的 `CommandPolicyRule` 和 `PolicyDecision` |
| **Goose** | permission inspector——工具调用前分层检查（Allow / Ask / Deny） | `BasicPolicyEngine` 的 `inspect_read` / `inspect_write` / `inspect_command` |

### 内置工具（Built-in Tools）

| 来源项目 | 借鉴了什么 | 落到 OpenTopia 哪里 |
|----------|-----------|-------------------|
| **Codex** | 一等工具设计（first-class tool）、apply_patch 作为独立工具 | `crates/opentopia-core/src/tools.rs` 的 `Tool` trait 和七个内置工具 |
| **Codex** | shell 命令的跨平台实现（Windows PowerShell vs Unix sh） | `ExecRequest::shell()` 的跨平台分支 |

### MCP 协议（Model Context Protocol）

| 来源项目 | 借鉴了什么 | 落到 OpenTopia 哪里 |
|----------|-----------|-------------------|
| **Goose** | extension manager——MCP 管理器作为 agent 能力扩展层 | `McpExtensionHost` 和 `McpStdioClient` |
| **Codex** | MCP 工具调用链路、权限路由、JSON-RPC over stdio | `mcp_host.rs` 的 request/response 模式 |

### 模型接入层（Model Provider）

| 来源项目 | 借鉴了什么 | 落到 OpenTopia 哪里 |
|----------|-----------|-------------------|
| **Goose** | provider 抽象层——统一模型调用接口，支持多种后端 | `crates/opentopia-core/src/provider.rs` 的 `ModelProvider` trait |
| **Goose** | tool_calls 解析和多轮工具结果回传 | `ModelRequest` 的 `tool_candidates` / `previous_tool_calls` / `tool_results` |
| **Codex** | 模型流式输出（streaming）、token 用量追踪 | `ModelProvider::stream` 和 `ModelStreamDelta` / `ModelUsage` |

### CLI 命令行

| 来源项目 | 借鉴了什么 | 落到 OpenTopia 哪里 |
|----------|-----------|-------------------|
| **Codex** | CLI 作为独立 crate 复用 core，不做重复逻辑 | `crates/opentopia-cli/src/main.rs` |

### 整体架构与组合方式

| 理念 | 来源 | 说明 |
|------|------|------|
| Electron 桌面壳 + Rust agent server + SQLite 持久化 + React UI | **Goose + opencode + Codex** 的组合借鉴 | 桌面壳借鉴 Goose（成熟 Electron 经验），Sidecar 方式借鉴 opencode，Rust runtime 借鉴 Codex，持久化借鉴 OpenHands |
| 事件驱动 UI（后端发事件，前端消费） | **Codex SQ/EQ** | 前端不推断状态，只消费事件 |
| 完整执行轨迹导出 | **OpenHands trajectory** | `GET /api/threads/:id/trajectory` 导出 messages/events/approvals/artifacts |

> 详细的源码借鉴映射和每个模块的落地对照表见 [docs/source-adaptation-map.md](file:///j:/Project/OpenTopia/docs/source-adaptation-map.md)。
