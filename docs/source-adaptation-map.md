# OpenTopia 源码借鉴与模块落地映射

这份文档回答一个很具体的问题：OpenTopia 这个 MVP 到底借鉴了哪些开源项目的哪些模块，以及这些设计落到了本仓库的哪些代码里。

我没有把 Codex、Goose、opencode、OpenHands 的源码直接复制进来，而是按模块抽取架构模式并重写到 OpenTopia。这样可以避免重复造轮子，也能避免把别人的 UI 像素、命名、协议细节和实现代码原样搬过来。

## 已读本地源码

| 项目 | 本地源码路径 | 借鉴重点 | OpenTopia 落地点 |
| --- | --- | --- | --- |
| Codex | `J:\Project\codex cli\codex\codex-rs\protocol\src\protocol.rs` | SQ/EQ 异步提交/事件队列、typed event payload、thread/session 概念 | `crates/opentopia-core/src/model.rs` 的 `AgentEventPayload`，`crates/opentopia-server/src/main.rs` 的 REST + SSE |
| Codex | `J:\Project\codex cli\codex\codex-rs\protocol\src\approvals.rs` | shell/apply_patch/network/MCP 等审批事件建模 | `ApprovalRequested` 事件、`/api/threads/{thread_id}/approvals/{approval_id}/decision` |
| Codex | `J:\Project\codex cli\codex\codex-rs\core\src\exec_policy.rs` | 命令风险判定、approval policy、exec policy amendment | `crates/opentopia-core/src/policy.rs` 的 `PermissionMode`、`PolicyDecision`、危险命令 ask |
| Codex | `J:\Project\codex cli\codex\codex-rs\core\src\apply_patch.rs` | 把 patch 当成一等工具，并进入权限链路 | `crates/opentopia-core/src/tools.rs` 的 `apply_patch` 工具与 `/patch` 命令 |
| Goose | `J:\Project\Goose\ui\desktop\src\main.ts` | Electron 主进程、后端服务启动、窗口生命周期、设置/菜单/更新的主进程承载 | `apps/desktop/electron/main.cjs` 的 Electron shell 和 Rust server auto-start |
| Goose | `J:\Project\Goose\crates\goose\src\agents\agent.rs` | agent loop、provider/tool/session 组合、frontend tool 分类 | `crates/opentopia-core/src/agent.rs` 的 AgentCore、provider、deterministic tools |
| Goose | `J:\Project\Goose\crates\goose\src\permission\permission_inspector.rs` | 工具调用前的 inspector/permission 分层，read-only/approval/deny 三态 | `crates/opentopia-core/src/policy.rs` 和 desktop approval card |
| Goose | `J:\Project\Goose\crates\goose\src\agents\extension_manager.rs` | extension/MCP 管理器作为 agent 能力扩展层 | MVP 暂留接口边界，下一步加入 `opentopia-extension-host` |
| opencode | `J:\Project\opencode\source\packages\desktop\src-tauri\src\server.rs` | desktop sidecar server spawn、health check、localhost no-proxy、WSL 配置 | `apps/desktop/electron/main.cjs` 的 `startBackendIfNeeded()`，后续加入 WSL path adapter |
| opencode | `J:\Project\opencode\source\packages\desktop\src-tauri\src\lib.rs` | sidecar 状态、初始化阶段事件、kill sidecar、平台命令桥 | `apps/desktop/electron/preload.cjs` 和 `window.opentopia` 平台桥 |
| opencode | `J:\Project\opencode\source\packages\desktop\src\index.tsx` | `PlatformProvider` 风格的桌面能力抽象：open path、dialog、storage、notification、OS 信息 | MVP 已实现 platform info/open external，后续扩展 dialog/storage/notification |
| opencode | `J:\Project\opencode\source\packages\ui\src\theme` | 可换主题、token 化 UI、工作台布局风格 | `apps/desktop/src/styles/app.css` 的 restrained workbench 视觉基底 |
| OpenHands | `J:\Project\openhand\openhands\app_server\event\event_service.py` | event service 抽象、event search/count/batch get | `SqliteSessionStore` 的 events 持久化与 `GET /api/threads/{thread_id}/events` |
| OpenHands | `J:\Project\openhand\openhands\app_server\event\event_router.py` | event router 与分页查询 API | MVP 先实现 since-based event listing，后续改 page cursor |
| OpenHands | `J:\Project\openhand\openhands\app_server\app_conversation\app_conversation_service.py` | sandboxed conversation service、start task 状态流、export trajectory | `Thread`/`Message`/`AgentEvent` 基础模型，后续实现 task state machine 和 trajectory export |
| OpenHands | `J:\Project\openhand\openhands\app_server\sandbox\sandbox_router.py` | sandbox 生命周期、pause/resume/delete、session-scoped secrets | 后续的 sandbox service/secret broker 设计来源 |
| Trae Work | 未在 `J:\Project` 找到本地源码 | 只能借鉴产品形态：三栏工作台、聊天驱动任务、文件/终端/审批并列 | `apps/desktop/src/App.tsx` 的 Codex/Trae-like 工作台布局 |

## 当前 MVP 的组合方案

OpenTopia 当前采用的是“Electron 桌面壳 + Rust 本地 agent server + SQLite 事件持久化 + React 工作台 UI”的组合。

核心原因：

- Electron 借鉴 Goose：跨平台桌面能力成熟，便于以后接入菜单、托盘、auto-update、原生通知、全局快捷键。
- Rust agent runtime 借鉴 Codex：本地工具执行、权限、patch、事件协议这些都更适合放在强类型后端。
- sidecar 运行方式借鉴 opencode：桌面 UI 不直接执行 agent，而是启动/连接本地 server，未来 CLI、Web、插件都可以复用同一套 API。
- sandbox/trajectory 借鉴 OpenHands：MVP 先本地执行，下一阶段把 Docker/remote sandbox 作为 runtime adapter 加进去。

## 已经落地的模块

| OpenTopia 模块 | 代码路径 | 来源模式 | 当前状态 |
| --- | --- | --- | --- |
| Desktop shell | `apps/desktop/electron/main.cjs` | Goose Electron main + opencode sidecar health check | 可启动窗口，可自动拉起 Rust server |
| Desktop platform bridge | `apps/desktop/electron/preload.cjs` | opencode PlatformProvider | 已有 platform info/open external，后续扩展 dialog/storage |
| Workbench UI | `apps/desktop/src/App.tsx` | Codex/Trae/opencode 三栏工作台 | thread list、chat、timeline、workspace/tool panel、approval card |
| Agent protocol | `crates/opentopia-core/src/model.rs` | Codex SQ/EQ typed events + OpenHands events | Thread、Message、ToolCall、AgentEvent |
| Local server | `crates/opentopia-server/src/main.rs` | OpenHands REST event service + Codex event stream | REST + SSE + approval decision |
| Session store | `crates/opentopia-core/src/store.rs` | OpenHands event persistence | SQLite thread/message/event |
| Policy engine | `crates/opentopia-core/src/policy.rs` | Codex exec_policy + Goose permission inspector | chat/read_only/auto/approve/full_access |
| Tools | `crates/opentopia-core/src/tools.rs` | Codex apply_patch/shell/read/write tool surface | list/read/write/shell/diff/apply_patch |
| Provider | `crates/opentopia-core/src/provider.rs` | Goose provider abstraction | OpenAI-compatible provider + mock fallback |
| CLI | `crates/opentopia-cli/src/main.rs` | Codex CLI split package | new/list/send |

## 还应该继续借鉴但暂未实现的模块

下一阶段优先级：

1. Codex exec policy：把当前简单危险字符串判断升级成 prefix rule、可持久化 approval、network policy、temporary grant。
2. Goose extension manager：实现 MCP extension host，支持 per-thread extension 列表、tool annotation、tool schema。
3. opencode desktop platform：补齐 open directory/file dialog、path opener、notification、deep link、WSL path adapter。
4. OpenHands sandbox：实现 sandbox runtime adapter，先 Docker，再 remote runtime；加入 session-scoped secret broker。
5. OpenHands trajectory export：支持按 thread 导出 events、messages、workspace diff、tool outputs。
6. Goose/Electron updater：补齐 auto-update、签名、崩溃日志、启动日志、single-instance 深链。

## 不建议直接复制的部分

- 不直接复制 Trae/Codex 的 UI 像素。三栏工作台、消息流、右侧工具面板属于通用产品范式，但具体 spacing、颜色、动效、图标组合需要形成 OpenTopia 自己的设计 token。
- 不直接复制 Codex 的完整 protocol。Codex 协议很强但很大，MVP 先保留核心 event shape，避免过早背上完整兼容负担。
- 不直接复制 OpenHands 的 Python server。OpenTopia 当前定位是桌面本地优先，Rust sidecar 更适合分发；OpenHands 更适合作为 sandbox/service 边界参考。
- 不把 opencode 的 Tauri 桌面壳照搬。用户偏好 Electron，且 Goose 也给了成熟 Electron 参考，所以本项目选 Electron。

## 许可证注意

当前实现是架构级借鉴与重写，不包含从上述项目复制的大段源码。后续如果引入任一项目的代码文件、协议 schema、UI 资源或图标资产，需要先把对应 license、NOTICE、attribution 和改动说明加入仓库。
