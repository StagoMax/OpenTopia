# OpenTopia 源码借鉴与模块落地映射

这份文档回答一个很具体的问题：OpenTopia 这个 MVP 到底借鉴了哪些开源项目的哪些模块，以及这些设计落到了本仓库的哪些代码里。

大多数模块按架构模式重写，没有复制其他项目的 UI 像素或大段实现代码。
一个明确例外是 Windows 沙箱：OpenTopia 直接分发并调用 Codex 的
restricted-token helper binaries，并随安装包附带其 Apache-2.0 `LICENSE`。
这属于有许可证约束的二进制复用，不应描述成单纯“借鉴思路”。

## 已读本地源码

| 项目 | 本地源码路径 | 借鉴重点 | OpenTopia 落地点 |
| --- | --- | --- | --- |
| Codex | `J:\Project\codex cli\codex\codex-rs\protocol\src\protocol.rs` | SQ/EQ 异步提交/事件队列、typed event payload、thread/session 概念 | `crates/opentopia-core/src/model.rs` 的 `AgentEventPayload`，`crates/opentopia-server/src/main.rs` 的 REST + SSE |
| Codex | `J:\Project\codex cli\codex\codex-rs\protocol\src\approvals.rs` | shell/apply_patch/network/MCP 等审批事件建模 | `ApprovalRequested` 事件、`/api/threads/{thread_id}/approvals/{approval_id}/decision` |
| Codex | `J:\Project\codex cli\codex\codex-rs\core\src\exec_policy.rs` | 命令风险判定、approval policy、exec policy amendment | `crates/opentopia-core/src/policy.rs` 的 `PermissionMode`、`PolicyDecision`、危险命令 ask |
| Codex | `J:\Project\codex cli\codex\codex-rs\core\src\apply_patch.rs` | 把 patch 当成一等工具，并进入权限链路 | `crates/opentopia-core/src/tools.rs` 的 `apply_patch` 工具与 `/patch` 命令 |
| Codex | `J:\Project\codex cli\codex\codex-rs\sandboxing/`、本机 Codex appserver helpers | OS 级沙箱：Linux bwrap、macOS Seatbelt、Windows restricted token/ACL/job object | `sandbox.rs` 三平台 adapter；Windows 直接调用并打包 `codex.exe`、command runner、sandbox setup helper 与 Apache-2.0 license |
| Goose | `J:\Project\Goose\ui\desktop\src\main.ts` | Electron 主进程、后端服务启动、窗口生命周期、设置/菜单/更新的主进程承载 | `apps/desktop/electron/main.cjs` 的 Electron shell 和 Rust server auto-start |
| Goose | `J:\Project\Goose\crates\goose\src\agents\agent.rs` | agent loop、provider/tool/session 组合、frontend tool 分类 | `crates/opentopia-core/src/agent.rs` 的 AgentCore、provider、deterministic tools |
| Goose | `J:\Project\Goose\crates\goose\src\permission\permission_inspector.rs` | 工具调用前的 inspector/permission 分层，read-only/approval/deny 三态 | `crates/opentopia-core/src/policy.rs` 和 desktop approval card |
| Goose | `J:\Project\Goose\crates\goose\src\agents\extension_manager.rs` | extension/MCP 管理器作为 agent 能力扩展层 | `McpExtensionHost`、`McpStdioClient`、线程级启停和 agent tool schema 注册 |
| opencode | `J:\Project\opencode\source\packages\desktop\src-tauri\src\server.rs` | desktop sidecar server spawn、health check、localhost no-proxy、WSL 配置 | `apps/desktop/electron/main.cjs` 的 `startBackendIfNeeded()`，后续加入 WSL path adapter |
| opencode | `J:\Project\opencode\source\packages\desktop\src-tauri\src\lib.rs` | sidecar 状态、初始化阶段事件、kill sidecar、平台命令桥 | `apps/desktop/electron/preload.cjs` 和 `window.opentopia` 平台桥 |
| opencode | `J:\Project\opencode\source\packages\desktop\src\index.tsx` | `PlatformProvider` 风格的桌面能力抽象：open path、dialog、storage、notification、OS 信息 | MVP 已实现 platform info/open external，后续扩展 dialog/storage/notification |
| opencode | `J:\Project\opencode\source\packages\ui\src\theme` | 可换主题、token 化 UI、工作台布局风格 | `apps/desktop/src/styles/app.css` 的 restrained workbench 视觉基底 |
| OpenHands | `J:\Project\openhand\openhands\app_server\event\event_service.py` | event service 抽象、event search/count/batch get | `SqliteSessionStore` 的 events 持久化与 `GET /api/threads/{thread_id}/events` |
| OpenHands | `J:\Project\openhand\openhands\app_server\event\event_router.py` | event router 与分页查询 API | MVP 先实现 since-based event listing，后续改 page cursor |
| OpenHands | `J:\Project\openhand\openhands\app_server\app_conversation\app_conversation_service.py` | conversation service、start task 状态流、export trajectory | `Thread`/`Message`/`AgentEvent` 模型和 trajectory export（含 approvals/artifacts/workspace diff） |
| OpenHands | `J:\Project\openhand\openhands\app_server\sandbox\sandbox_router.py` | sandbox 生命周期、pause/resume/delete、session-scoped secrets | Docker/remote sandbox 生命周期管理和 secret broker 设计来源（如后续引入） |
| Trae Work | 未在 `J:\Project` 找到本地源码 | 只能借鉴产品形态：三栏工作台、聊天驱动任务、文件/终端/审批并列 | `apps/desktop/src/App.tsx` 的 Codex/Trae-like 工作台布局 |

## 当前 MVP 的组合方案

OpenTopia 当前采用的是"Electron 桌面壳 + Rust 本地 agent server + SQLite 事件持久化 + React 工作台 UI"的组合。

核心原因：

- Electron 借鉴 Goose：跨平台桌面能力成熟，便于以后接入菜单、托盘、auto-update、原生通知、全局快捷键。
- Rust agent runtime 借鉴 Codex：本地工具执行、权限、patch、事件协议这些都更适合放在强类型后端。
- sidecar 运行方式借鉴 opencode：桌面 UI 不直接执行 agent，而是启动/连接本地 server，未来 CLI、Web、插件都可以复用同一套 API。
- 沙箱采用 Codex 同类 OS 原生路线：Linux bubblewrap、macOS Seatbelt、
  Windows 直接复用 Codex restricted-token backend。Docker/Remote 按当前决定延期。
- trajectory 借鉴 OpenHands：MVP 已支持按 thread 导出 events/messages/approvals/artifacts。

## 已经落地的模块

| OpenTopia 模块 | 代码路径 | 来源模式 | 当前状态 |
| --- | --- | --- | --- |
| Desktop shell | `apps/desktop/electron/main.cjs` | Goose Electron main + opencode sidecar health check | 可启动窗口，可自动拉起 Rust server |
| Desktop platform bridge | `apps/desktop/electron/preload.cjs` | opencode PlatformProvider | platform info、目录选择、recent workspace、open path、safeStorage metadata/set/delete、logs |
| Workbench UI | `apps/desktop/src/App.tsx` | Codex/Trae/opencode 三栏工作台 | thread、chat、timeline、Monaco、xterm、artifact、staged/unstaged hunk review |
| Agent protocol | `crates/opentopia-core/src/model.rs` | Codex SQ/EQ typed events + OpenHands events | Thread、Message、ToolCall、AgentEvent |
| Local server | `crates/opentopia-server/src/main.rs` | OpenHands REST event service + Codex event stream | REST + SSE + approval decision |
| Session store | `crates/opentopia-core/src/store.rs` | OpenHands event persistence | SQLite thread/message/event/approval/artifact/settings/MCP/terminal history |
| Policy engine | `crates/opentopia-core/src/policy.rs` | Codex exec_policy + Goose permission inspector | chat/read_only/auto/approve/full_access |
| Tools | `crates/opentopia-core/src/tools.rs` | Codex apply_patch/shell/read/write tool surface | list/read/write/shell/diff/apply_patch |
| Provider | `crates/opentopia-core/src/provider.rs` | Goose provider abstraction | OpenAI-compatible provider、tool_calls 解析、多轮工具结果回传、mock fallback |
| CLI | `crates/opentopia-cli/src/main.rs` | Codex CLI split package | new/list/send |
| Execution environment | `crates/opentopia-core/src/execution.rs` | Codex exec/sandbox boundary | 路径收敛、超时/取消/输出限制、sandbox command plan、stdio session |
| OS sandbox | `crates/opentopia-core/src/sandbox.rs` | Codex 三平台隔离模型；Windows helper binary 直接复用 | read-only/workspace-write/danger-full-access、writable roots、受保护元数据、独立 backend enforcement、network policy；Windows unelevated 已做真实越界写拒绝测试 |
| Persistent terminal | `crates/opentopia-server/src/main.rs`、`XtermTerminal.tsx` | Codex terminal/exec 生命周期模式 + `portable-pty`/xterm.js | 每 thread 长驻 PTY、raw input、resize、SSE、SQLite 回放、Windows process-tree close |
| Context compaction | `crates/opentopia-server/src/main.rs`、`agent.rs` | coding-agent durable summary 模式 | 真实 provider 摘要、metadata、latest-summary 恢复及后续 turn 注入 |
| MCP host | `mcp_host.rs`、`mcp.rs` | Goose extension manager + MCP JSON-RPC | initialize/list_tools/call_tool、schema cache、policy、bounded agent loop |

## 还应该继续借鉴但暂未实现的模块

下一阶段优先级：

1. Linux/macOS 沙箱：在对应原生发布机跑 bubblewrap/Seatbelt confinement 集成测试，并增加 seccomp/Landlock 与资源配额。
2. Goose extension ecosystem：在现有 MCP host 上增加 schema cache 持久化和产品化 GitHub/Linear/Jira/browser/document 连接器。
3. opencode desktop platform：继续补 notification、完整 deep link 路由、WSL path adapter。
4. OpenHands trajectory：补 tool output 的更完整序列化和 replay tooling。
5. Provider context：接入 provider-reported token usage 和自动压缩阈值。
6. **Docker/Remote 沙箱（明确延期）**：仅保留 `ExecutionEnvironment` 扩展点，当前不实现运行时。

## 不建议直接复制的部分

- 不直接复制 Trae/Codex 的 UI 像素。三栏工作台、消息流、右侧工具面板属于通用产品范式，但具体 spacing、颜色、动效、图标组合需要形成 OpenTopia 自己的设计 token。
- 不直接复制 Codex 的完整 protocol。Codex 协议很强但很大，MVP 先保留核心 event shape，避免过早背上完整兼容负担。
- 不直接复制 OpenHands 的 Python server。OpenTopia 当前定位是桌面本地优先，Rust sidecar 更适合分发。
- 不把 opencode 的 Tauri 桌面壳照搬。用户偏好 Electron，且 Goose 也给了成熟 Electron 参考，所以本项目选 Electron。
- Linux/macOS sandbox adapter 是按 OpenTopia 的 `ExecutionEnvironment` 重写；
  Windows 不重写安全敏感的 token/ACL/job-object 细节，而是通过稳定 CLI 边界
  直接复用 Codex helper。构建脚本必须同时打包对应 Apache-2.0 license。

## 沙箱路线与优先级

OS 级沙箱是本地开发的首选方案。工作现场在宿主机（IDE、依赖管理、git config），
OS 原生安全机制直接在用户工作区执行，无需完整容器环境。启动成本由平台后端
决定，不能统一承诺 `<10ms`；Windows Codex sandbox 冷启动明显高于普通子进程，
调用方和测试必须按实际延迟设置超时。

这里借鉴的不只是平台名称，还包括 Codex 的两轴模型：SandboxMode 定义文件系统/
网络技术边界，approval policy 决定跨边界时是否暂停。OpenTopia 不把 `never` 审批
解释为关闭沙箱；额外目录通过 writable roots 扩展。可写根下的 `.git`、`.agents`、
`.codex` 默认保护模式同样来自 Codex 的 `WritableRoot` 设计。Linux/macOS adapter
仍是 OpenTopia 重写，尚未复制 Codex 的 seccomp/Landlock 纵深层。

Docker/Remote 沙箱作为后续方向，用于固定 CI 环境、多租户部署等需要完整容器隔离的场景。前期聚焦 OS 级本地沙箱，架构上保留 Docker/Remote 通过同一 `ExecutionEnvironment` trait 接入的扩展点。

## 许可证注意

除 Windows Codex sandbox helpers 外，当前实现是架构级借鉴与重写，不包含从上述
项目复制的大段源码。Windows helper 的许可证由 `scripts/build-desktop.ps1` 强制查找
并复制到 `resources/codex-sandbox/LICENSE`；缺失时构建失败。后续如果引入其他项目
的代码文件、schema、UI 资源或图标资产，也必须先加入对应 license、NOTICE、
attribution 和改动说明。
