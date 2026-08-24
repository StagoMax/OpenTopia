# OpenTopia 全项目根因与化繁为简审查

审查日期：2026-08-24
审查对象：`J:\Project\OpenTopia` 当前工作树
审查目标：找出“为局部症状不断补丁、但根因应在更高层解决”的实现，并给出能减少代码、状态面和运行成本的替代边界。

> 重要说明：本报告审查的是一个正在快速变化的工作树。审查期间 `git status --short` 从此前的约 62 项增长到 149 项，因此行号和行数是 2026-08-24 审查快照，不是发布版本的永久常量。报告没有修改业务代码，也没有把工作树中已有改动归因于本次审查。

## 1. 结论先行

上一轮只列出少量问题，不是因为项目只有少量问题，而是只汇报了几项最高收益的根因。扩大审查后，共整理出：

- 2 项 P0：安全或配置边界错误，应立即处理；
- 14 项 P1：高概率继续放大复杂度、性能或一致性风险，应进入近期架构收敛；
- 14 项 P2：迁移尾巴、重复机制和工程护栏缺口，适合按领域成批清理；
- 另有若干大模块属于问题域本身复杂，不能仅凭行数粗暴拆分。

项目当前不是“到处都有独立小毛病”，而是少数几个根因在多个模块反复制造补丁：

1. **一个事实有多个权威表示**：AgentTurn 与产品 Turn、MCP Server 与 IntegrationDefinition、WorkForm 与 GoalRecord、数据库索引列与 `document_json` 都在承担相同事实的一部分。
2. **边界只有 TypeScript/Rust 类型，没有统一运行时协议**：HTTP、Electron IPC、SSE、Flow 条件和 JSON Schema 都出现“看起来有契约、实际仍靠断言或手写子集”的情况。
3. **持久化模型与执行模型没有分离**：Flow 每个并行结果都回写整份运行文档；会话与上下文热路径反复加载完整历史。
4. **迁移以叠加为主、删除为辅**：CSS 新旧层同时加载，旧 API 向新控制面双写，旧 prompt builder 以 `allow(dead_code)` 保留，renderer 启动时继续做数据迁移。
5. **根组件和宽接口承担过多协调责任**：`App.tsx`、`SessionStore`、`ToolInvocationContext`、Agent provider loop 都在用参数、状态和条件分支补偿缺失的领域边界。
6. **验证脚本给出了比实际更强的“全绿”印象**：根 `pnpm check` 不执行 Rust 测试和 Electron 测试，设计系统检查也只扫描 UI primitives 与 `ui.css`。

因此，最有收益的方向不是逐个缩短大文件，而是收敛以下五个系统级边界：

- 单一权威状态 + 明确投影；
- 生成的端到端协议 + 运行时解码；
- 追加式日志/检查点 + 可恢复调度；
- 分页读模型 + 增量同步；
- 按领域拥有 UI 状态、样式和能力端口。

## 2. 当前代码行数

### 2.1 统计口径

这里使用物理行数，同时给出非空行作为参考。统计当前工作树，而不是只统计 Git 已跟踪文件，因此会包含本地新增源码。排除：

- `node_modules/`、`target/`、运行时缓存、构建输出；
- `.opentopia/`、`.codex-tmp*`、`.tmp-*`、日志、数据库和二进制文件；
- lockfile、用户数据和 Office 文件；
- 嵌套 explorer 应用的 `.next/`、`dist/`、`build/`、`node_modules/`。

Rust 生产文件内的 `#[cfg(test)]` 单元测试仍计入生产文件，因为物理行统计无法在不解析语法树的情况下可靠拆开。独立 `tests/` 和 `tests.rs` 文件单独列出。

### 2.2 主工程代码

| 类别 | 文件数 | 物理行 | 非空行 |
|---|---:|---:|---:|
| Rust 生产源码（含同文件单测） | 274 | 169,819 | 160,222 |
| Rust 独立测试文件 | 44 | 22,343 | 20,949 |
| Desktop renderer 手写生产代码 | 295 | 96,440 | 88,924 |
| Desktop renderer 测试 | 86 | 10,452 | 9,532 |
| Electron host 生产代码 | 16 | 8,493 | 7,975 |
| Electron 测试 | 6 | 739 | 697 |
| Desktop 构建辅助代码 | 5 | 313 | 293 |
| Evaluation runtime/adapters/tools | 20 | 6,773 | 6,420 |
| Evaluation 测试 | 13 | 1,062 | 993 |
| 仓库脚本 | 58 | 11,313 | 10,531 |
| 嵌入 Rust 的 prompt Markdown | 18 | 225 | 172 |
| **手写主工程合计** | **835** | **327,972** | **306,708** |

补充口径：

| 补充项 | 文件数 | 物理行 |
|---|---:|---:|
| Evaluation 示例/fixture 源码 | 88 | 2,835 |
| Desktop 生成契约（TS、JSON Schema、fixture） | 12 | 25,542 |
| `shared/` 与 evaluation 声明式 schema | 7 | 517 |
| 嵌套 `apps/agent-loop-explorer` | 18 | 1,827 |
| 嵌套 `apps/agent-loop-explorer-copy` | 18 | 1,827 |

由此得到几个不会混淆含义的总数：

- **主工程手写源码、测试、工具：327,972 行**；
- 加入 evaluation 示例/fixture：**330,807 行**；
- 再加入生成契约和声明式 schema，主工程代码类文本：**356,866 行**；
- 再把两份嵌套 explorer 应用都算入：**360,520 行**。

文档方面：

- 只算 `docs/`：51 个文件，36,988 行；
- 算入根目录、`design-system/`、evaluation fixture 文档等一方 Markdown，但不重复计算 prompt：261 个文件，61,103 行；
- 所以“全部代码类文本 + 全部一方 Markdown”约为 **421,623 行**。

最适合回答“项目代码多少”的数字是 **约 35.7 万行主工程代码类文本**；若只关心人工维护成本，则应看 **约 32.8 万行手写代码**，不要让 2.55 万行生成契约放大判断。

### 2.3 模块规模

| 模块 | 文件数 | 物理行 |
|---|---:|---:|
| `crates/opentopia-core` | 243 | 147,026 |
| `crates/opentopia-server` | 59 | 37,292 |
| `crates/opentopia-windows-sandbox` | 14 | 7,495 |
| `crates/opentopia-sandbox-protocol` | 1 | 229 |
| `crates/opentopia-cli` | 1 | 120 |
| `apps/desktop/src/components` | 129 | 37,662 |
| `apps/desktop/src/styles` | 19 | 19,282 |
| `apps/desktop/src/api` | 18 | 8,625 |
| `apps/desktop/src/features` | 32 | 8,316 |
| `apps/desktop/electron` | 22 | 9,232 |
| `evaluation/src` | 12 | 4,079 |

排除生成契约和常规 `*.test.*` 后，仍有 **68 个手写文件超过 1,000 行，225 个超过 500 行**。这只是审查信号，不代表 68 个文件都必须拆；是否拆分应看责任是否混杂、是否存在并行权威和是否能建立稳定接口。

当前最大的手写文件包括：

| 文件 | 行数 | 判断 |
|---|---:|---|
| `apps/desktop/src/App.tsx` | 4,841 | 明显是应用协调器、领域状态和 UI 布局混合 |
| `crates/opentopia-server/src/main.rs` | 3,508 | bootstrap、Turn 执行、上下文和投影协调混合 |
| `crates/opentopia-core/src/flow_runtime.rs` | 3,111 | 执行器、调度、持久化协议和表达式解释混合 |
| `crates/opentopia-core/src/store.rs` | 2,882 | 多个存储领域集中于一个实现入口 |
| `crates/opentopia-core/src/agent.rs` | 2,879 | Agent 配置、生命周期、工具编排和兼容逻辑混合 |
| `crates/opentopia-core/src/enterprise.rs` | 2,653 | 模板领域、schema 验证和发布逻辑混合 |
| `apps/desktop/electron/main.cjs` | 2,652 | 进程生命周期、IPC、安全校验、运行时管理混合 |
| `crates/opentopia-server/src/turn_changes.rs` | 2,608 | 复杂但领域较集中，拆分收益需谨慎评估 |
| `apps/desktop/src/styles/app-workspace-legacy.css` | 2,469 | 迁移覆盖层，不应继续扩展 |
| `apps/desktop/electron/browser-host.cjs` | 2,451 | 浏览器状态机与 Electron 控制混合 |

## 3. P0：立即处理

### P0-01 Electron 日志读取使用字符串前缀判断目录边界

**证据**：`apps/desktop/electron/main.cjs:2301-2308`

`logs:read` 对 renderer 传入的任意路径做 `path.resolve`，再用：

```js
resolvedPath.startsWith(path.resolve(logsDirPath || ""))
```

判断它是否位于日志目录。字符串前缀不是路径包含关系。例如日志目录为 `C:\logs` 时，`C:\logs-elsewhere\file.txt` 仍以前者开头。在 Windows 上还要考虑大小写、分隔符、UNC 与符号链接/重解析点语义。

**根因**：IPC 暴露的是宿主文件路径，安全边界却由零散 handler 自行校验；renderer 知道并回传了不该成为能力凭据的绝对路径。

**建议**：

1. 短期使用 `path.relative(root, candidate)`，拒绝绝对 relative、`..`、空 root，并以规范化后的路径组件判断；
2. 日志列表返回不透明 `logId`，主进程保存 `logId -> canonical path` 映射，读取 API 只接受 ID；
3. 把 IPC 参数校验放进统一 channel schema；
4. 增加 sibling-prefix、大小写、UNC、`..`、symlink/junction 测试。

这比继续给 `startsWith` 增加分隔符特判更少代码，也更安全。

### P0-02 Provider 核心会扫描相邻项目的 `.env` 并接受业务专用别名

**证据**：

- `crates/opentopia-core/src/provider.rs:1320-1375`
- `crates/opentopia-core/src/provider/openai.rs:238-258`

Provider 初始化除当前目录 `.env` 外，还会构造一个中文“信贷审核助手”项目名并扫描同级目录，只要文件包含 `CREDIT_REVIEW_LLM_API_KEY` 或 `AUDIT_COPILOT_LLM_API_KEY` 就加载；OpenAI adapter 也把这些业务变量当作通用凭据别名。

**根因**：某次本地集成/评测的启动便利逻辑进入了产品核心 credential resolution。它既越过仓库边界，又让无关项目的凭据和模型配置影响 OpenTopia。

**影响**：意外读取其他项目秘密、配置来源不可解释、测试与生产行为依赖工作目录布局，且用户可能在 UI 未配置 provider 时仍调用到旁边项目的账号。

**建议**：

- Core 只接受显式 `ProviderSettings`；
- `.env` 加载只存在于 CLI/desktop bootstrap，并限定当前项目或显式 `OPENTOPIA_ENV_FILE`；
- 删除业务专用别名；如确需兼容，放进带版本和告警的一次性迁移层；
- provider status 返回 credential source（只返回来源类型，不返回秘密），便于诊断。

## 4. P1：高收益根因

### P1-01 “JSON Schema”实际是多个不一致的手写子集

**证据**：

- `crates/opentopia-core/src/enterprise.rs:1527`：`validate_schema_shape`
- `crates/opentopia-core/src/enterprise.rs:1665`：`validate_value_against_schema`
- `crates/opentopia-core/src/flow.rs:801`：字符串黑名单式 `safe_condition`
- `crates/opentopia-core/src/flow.rs:823`：浅层 `schemas_compatible`
- `evaluation/src/validation.mjs`：另一份手写 validation

模板发布、Flow 编译、运行时输入和 evaluation 使用了名称相近但支持关键字不同的验证器。调用者容易把“通过验证”理解为完整 JSON Schema 语义，实际上 `oneOf`、`allOf`、条件 schema、引用、数值边界等支持并不统一。

**更深的问题**：Flow 条件先靠字符黑名单判“安全”，执行时又由简化解释器解析；合法但不支持的表达式最终可能只得到 `false`。这把编译错误变成业务分支选择。

**建议**：

- 选定明确 draft 的标准 schema validator，Core 和 evaluation 共享同一组合规 fixture；
- Flow 条件在发布时编译为版本化 AST，运行时只解释 AST；
- 返回 `Result<bool, ConditionError>`，绝不把解析失败静默当 `false`；
- schema compatibility 需要结构化诊断，不再只给布尔值。

### P1-02 SSE receiver 落后时静默丢弃 durable event

**证据**：

- `crates/opentopia-server/src/event_bus.rs:15-30`：activity 1024、thread 256 的 broadcast buffer；
- `crates/opentopia-server/src/events_api.rs:43-44`：`result.ok()`；
- `crates/opentopia-server/src/events_api.rs:261-281`：live stream 对所有 `Err` 返回 `None`；
- `crates/opentopia-server/src/terminal_api.rs:348`：terminal stream 也丢弃错误。

`tokio::broadcast` 的 `Lagged` 表示消费者错过了事件。当前实现保持 SSE 连接但吞掉错误，客户端只会看到 sequence gap；若客户端不主动识别，界面可能永久缺失状态转移。

**根因**：durable event log 与低延迟 broadcast 被拼接成一个流，但没有定义“落后后如何重新进入 durable log”的协议。

**建议**：检测 `Lagged` 后按最后确认的 seq 从 SQLite 补放，或发送显式 `resync_required` 并关闭连接，让客户端带 `Last-Event-ID` 重连。给 sequence gap 增加端到端测试。EventBus 的 per-thread sender map 也应在无 receiver/线程归档后清理。

### P1-03 Flow 运行由 detached task 驱动，失败终态又是 best-effort

**证据**：`crates/opentopia-core/src/flow_runtime.rs:655-679`

`spawn_flow_run` 直接 `tokio::spawn`；`drive_flow_run` 失败后再加载运行记录并尝试 `update_flow_run`，但该更新结果被 `let _ = ...` 忽略。CAS 冲突或数据库故障时，运行可能长期留在非终态。

**根因**：长期运行的 durable workflow 被当作进程内 background future，而不是有 owner、lease、重试和恢复语义的 durable job。

**建议**：建立受监督的 Flow scheduler：持久化 claim/lease、节点边界续租、失败转换重试、启动恢复和明确 ownership。API 只提交/唤醒运行，不直接 spawn。这样取消、重启、并发和失败都由同一状态机处理。

### P1-04 Flow 并行节点结果逐个重写整份 JSON 运行文档

**证据**：

- `crates/opentopia-core/src/flow_runtime.rs:1056-1066`
- `crates/opentopia-core/src/flow_runtime.rs:1385-1425`
- `crates/opentopia-core/src/store.rs:839-843`

每个 pending write 都重新读取 `FlowRunV1`，clone/追加/sort，然后对完整 `document_json` 做 revision CAS。并行度越高，CAS 重试和 JSON 序列化成本越高；运行历史变大后，每个小结果的写放大也随之增大。

**建议**：把 checkpoint、node attempt、pending write 建成追加表，以 `(run_id, checkpoint_id, node_id, attempt)` 唯一约束保证幂等；提交 superstep 时在短事务中更新 run cursor/summary。完整运行文档变成读模型或快照，不再是每个节点写入的互斥单元。

### P1-05 Root Turn 同时维护 canonical AgentTurn 和产品 Turn

**证据**：`crates/opentopia-server/src/turns.rs:22-29, 154-156, 327-396`

代码已经明确声明 AgentTurn authoritative，但仍需要：

- 先提交 canonical，再更新 product projection；
- product resume 失败后手工回滚 canonical；
- status 读取时合并 canonical 状态；
- preflight 失败又允许 product-only Turn。

这已经不是简单投影，而是双写状态机和补偿事务。

**建议**：所有执行生命周期只写 AgentTurn；产品 Turn API 由 SQL view/read model 派生，产品特有字段单独存储。若 preflight 也需要 UI identity，就定义 canonical `rejected_before_execution` 状态，而不是另一套 product-only 生命周期。

### P1-06 会话和上下文热路径反复读取完整历史

**证据**：

- `crates/opentopia-server/src/context_api/turn_context.rs:506` 使用 `list_events(thread_id, None).unwrap_or_default()` 查最近 WorldState；
- 同文件 `:681-683` 在准备 Turn 时再次加载全部消息和事件；
- `crates/opentopia-server/src/main.rs:2620, 2658, 2729, 2744` 多处完整历史扫描；
- `crates/opentopia-server/src/message_api.rs:413` 为按 ID 找消息而加载整个线程；
- `crates/opentopia-core/src/store/session_store.rs:438` 计算 context budget 时加载全部消息。

不仅复杂度是 O(history)，`unwrap_or_default()` 还会把数据库失败伪装成“没有历史”，可能生成重复 snapshot 或错误上下文。

**建议**：增加 `get_message(id)`、`latest_event_by_kind`、`latest_context_checkpoint`、聚合 token/message counters 等窄查询；对长历史使用 cursor/page。数据库错误必须传播或进入显式 degraded 状态，不能回落为空历史。

### P1-07 Renderer 首次加载和缓存也保留完整消息/事件

**证据**：

- `apps/desktop/src/conversationSessionController.ts:269-284`：先 `listMessages`，再 `listConversationEvents`；
- `apps/desktop/src/conversationSession.ts:121-126, 306-313`：merge 后持续保留；
- `apps/desktop/src/conversationSessionController.ts:437`：默认缓存 8 个 controller；
- `apps/desktop/src/App.tsx:472`：显式传入 8。

先显示 messages 再加载 diagnostics 是合理的局部优化，但仍没有解决“两个无界列表全部载入并常驻”的根因。

**建议**：服务端提供 conversation read model（分页消息、当前 Turn、必要 tool 状态）；diagnostic event timeline 单独按需分页；renderer 只保留可见窗口和少量前后缓冲，旧消息由 cursor 加载。

### P1-08 一个 64-bit FNV fingerprint 同时用于缓存、身份、幂等和完整性

**证据**：

- `crates/opentopia-core/src/model_context.rs:798-804`：无版本 64-bit FNV-1a；
- Flow immutable hash：`crates/opentopia-core/src/flow.rs:412`；
- Agent template immutable content：`crates/opentopia-core/src/enterprise.rs:915`；
- 文件 stale-write 检测：`crates/opentopia-core/src/tools/filesystem_tool.rs:313, 378`；
- checkpoint blob 去重：`crates/opentopia-core/src/store.rs:2853`；
- workflow input/idempotency：`crates/opentopia-core/src/workflow_automation.rs:458`。

快速非密码 fingerprint 用于 cache telemetry 没问题，但不应承担 durable identity、完整性和幂等键。项目其他模块已经使用 `sha256:`，说明当前语义不统一。

**建议**：建立两个类型而不是一个字符串函数：

- `FastFingerprint`：只允许缓存分桶和遥测；
- `ContentDigest`：版本化 `sha256:<hex>` 或 `blake3:<hex>`，用于持久化身份、完整性、stale-write 和幂等。

同时统一 canonical JSON；旧 hash 通过带算法前缀的兼容读取迁移。

### P1-09 Browser route 切换的回滚错误被忽略

**证据**：`crates/opentopia-core/src/browser_router.rs:60-82`

切换 runtime 时先关闭旧 session，再创建新 session。新建失败后尝试在 previous runtime 重建，但结果被忽略；函数只返回原始新建错误。此时 route binding 仍可能指向 previous，但 previous session 实际并未恢复。

**建议**：优先做两阶段 handoff：新 runtime 准备成功后再关闭旧 runtime；做不到时，返回包含 primary 与 rollback error 的显式复合状态，并将 routing table 标记为 unavailable，而不是假装旧 route 可用。

### P1-10 Electron IPC 契约在四个表面手工同步

**证据**：

- `apps/desktop/electron/main.cjs`：34 个 `ipcMain.handle`；
- `apps/desktop/electron/preload.cjs`：47 个 `ipcRenderer.invoke` 调用点；
- `apps/desktop/src/types/platform.ts:297-433+`：手写 `window.opentopia` 接口；
- `apps/desktop/src/platform.ts`：renderer wrapper，`loadPlatformInfo` 最终 `as PlatformInfo`。

main handler、preload bridge、global type 和 renderer wrapper 可以独立漂移，运行时参数校验也分散在 handler 中。P0-01 正是这种边界形态产生的具体漏洞。

**建议**：定义单一 `IpcChannelMap`：channel、request schema、response schema、授权类型、sender/window 限制；从它生成 preload 和 TS 类型，main 使用统一 `registerValidatedHandler`。浏览器 host 可以保留独立子协议，但也应使用同一机制。

### P1-11 HTTP 生成契约与手写 domain types 大量重名，transport 仍允许任意泛型

**证据**：

- 生成文件 `apps/desktop/src/api/generated/desktop-http-v1.generated.ts` 有 412 个导出声明；
- 手写 renderer 有 610 个导出声明；
- 两边有 **173 个完全同名声明**；
- `apps/desktop/src/api/client/transport.ts:35, 48` 暴露 `get<T>`/`post<T>`；
- `apps/desktop/src/api/httpContracts.ts:33` 解码后仍返回 `value as T`；
- ApiClient 通过 `Configuration -> Extensions -> Conversation -> Workspace -> Mcp -> Connections` 继承链聚合。

生成 response schema 是进步，但 request、method/path key 与 domain mapping 仍是手写的；任意 `T` 使调用点可以声明一个与真实 endpoint 无关的类型。coverage 脚本只能证明某路径“看起来被覆盖”，不能证明泛型类型相等。

**建议**：transport API 只接受生成的 endpoint key，例如 `request<K extends EndpointKey>(key, input): Promise<ResponseOf<K>>`；request/response 都生成并运行时解码。domain type 只在确有语义变换时存在，并用显式 mapper 命名。ApiClient 改为 composition 的领域 client，不用线性继承链表达“拥有多个 API”。

### P1-12 `App.tsx` 是全应用状态总线，已有局部性能补丁

**证据**：

- `apps/desktop/src/App.tsx` 4,841 行；
- 28 个 `useState`、39 个 `useEffect`；
- `RightPanel` 调用约 124 个 props（`apps/desktop/src/App.tsx:4246+`）；
- `loadThreadActivityTurnStatuses` 对每个 thread 单独请求状态（`:244-259`）；
- 启动时执行 legacy project migration（`:1519`，实现位于 `:4558+`）。

大量 memo、selector、独立 conversation controller 和分阶段加载是在补偿根组件每次事件都可能重新协调整个 shell。N+1 status 请求也是缺少 thread activity read model 的表现。

**建议**：按领域拆 ownership，而不只是拆 JSX 文件：

- `WorkspaceSession`：项目、工作区和文件状态；
- `ConversationSession`：消息、Turn、SSE 和 composer；
- `WorkbenchSession`：预览、终端、diff；
- `IntegrationSession`：plugins、MCP、connections；
- App shell 只组合 session selector 和布局。

服务端提供批量 thread activity endpoint，数据迁移移到版本化 server/store migration，不在 renderer 启动路径逐次探测。

### P1-13 CSS 迁移层全部同时生效，行为依赖 import order

**证据**：`apps/desktop/src/styles/app.css` 同时导入：

- `app-foundation.css`（2,425 行）；
- `app-legacy-layout.css`（1,955 行）；
- `app-workspace-legacy.css`（2,469 行）；
- `app-workspace.css`（1,699 行）；
- 以及其他 feature styles。

粗略 selector token 检查发现 **305 个 selector token 出现在多个 CSS 文件**。典型重复：

- `.message-list`：foundation `:519`、legacy-layout `:842`、workspace-legacy `:1316`；
- `.composer`：foundation `:593`、legacy-layout `:1240`、workspace-legacy `:1773`；
- `.approval-card`：foundation `:694`、legacy-layout `:1656`、workspace-legacy `:2170`。

这不是单纯重复行；所有层同时加载，最终行为依赖顺序和 specificity，修改一处时只能再补一层覆盖。

**建议**：建立 selector ownership 清单，按 conversation、workspace、settings 等 feature 迁移；每完成一个 feature 就删除旧层对应 selector，最终整层删除 legacy 文件。新样式使用 scoped component/feature root，CI 检查跨 owner selector 重复，而不是继续追加 override。

### P1-14 `pnpm check` 不是完整质量门

**证据**：

- 根 `package.json` 的 `check` 执行 `cargo check --workspace`，不执行 `cargo test --workspace`；
- desktop `test` 只执行 `src/**/*.test.ts`；
- 6 个 Electron 测试（739 行）只能通过单独 `test:electron` 执行；
- `check` 没有 `cargo fmt --check`、`cargo clippy` 或 Prettier check；
- 未发现仓库内 `.github` CI workflow。

上一轮审查快照上，手工执行的 `pnpm check` 曾通过，完整 `cargo test --workspace` 也曾得到 1,178 passed、2 ignored、0 failed；但这两个事实不是同一质量门，而且本轮工作树继续发生了大量变化。

**建议**：

- `check:fast`：cargo check、TS typecheck、targeted tests、contract/design/boundary；
- `check:full`：fmt、clippy、完整 cargo test、renderer tests、Electron tests、evaluation；
- CI 必须跑 `check:full`；本地 pre-push 可选；
- 报告必须显示每一类测试数量，避免一个总的绿色退出码掩盖漏跑。

## 5. P2：成批收敛的复杂度

### P2-01 `SessionStore` 是 124 方法的总线接口

**证据**：`crates/opentopia-core/src/store/session_store.rs:23+`

单个 trait 横跨 project、thread、message、event、effect、terminal、artifact、provider state、Flow、MCP、plugin、goal 等领域。调用者容易拿到超出所需的能力，测试替身实现成本高，新增领域会继续扩接口。

**建议**：按能力拆成窄 port，例如 `ConversationReader`、`EventLog`、`TurnRepository`、`FlowRunRepository`、`EffectJournal`。组合只发生在 bootstrap；tool、agent、flow runtime 只拿所需能力。不要一次性机械拆 124 个方法，可从 Flow 和 Agent 执行热路径开始。

### P2-02 Agent provider loop 用 20 多个参数传递隐式状态机

**证据**：

- `crates/opentopia-core/src/agent/provider_turn_loop.rs:16+`
- `crates/opentopia-core/src/agent/provider_round.rs:22+`
- 多处 `#[allow(clippy::too_many_arguments)]`。

参数中同时包含 immutable request、mutable conversation、预算计数器、tool queues、provider response items、compatibility hash 和 events。参数爆炸不是风格问题，而是 Turn state 没有一等领域表示。

**建议**：引入 `ProviderTurnState`（可变、可持久化/可测试）和 `TurnServices`（不可变依赖），round 输入输出使用显式 transition enum。这样暂停/恢复、压缩、工具并行和 completion guard 都对同一状态机操作。

### P2-03 `ToolInvocationContext` 是 32 字段的能力袋

**证据**：`crates/opentopia-core/src/tools.rs:60-123`

其中大量字段为 `Option`，不同工具靠“有没有某字段”推断运行模式。虽然近期已用 `ToolStateStore` 缩窄一部分持久化能力，但 browser、computer、MCP、collaboration、artifact、Flow、goal 等仍装在同一个 context。

**建议**：保留一个最小 `InvocationCore`，其他能力以 typed capability set/子上下文显式注入。工具注册时声明 capability requirements，执行前统一校验，不让每个工具重复 Option 分支。

### P2-04 新旧 Turn 执行路径在 server main 中平行增长

**证据**：

- `crates/opentopia-server/src/main.rs:1248`：`run_new_agent_turn`；
- `crates/opentopia-server/src/main.rs:1966`：`run_resumed_agent_turn`。

两条路径分别协调 workspace lock、change capture、provider/settings、collaboration snapshot、goal finalization 和 finish。差异有必要，但公共生命周期散落会导致新增行为只补到其中一条。

**建议**：统一为 `PreparedRootTurn` + `RootTurnExecution`，new/resume 只负责构造不同的 continuation source；执行、终态和 side effects 共享一条 transition pipeline。

### P2-05 MCP legacy API 向 Connections 控制面双写，删除路径不对称

**证据**：

- `crates/opentopia-server/src/mcp_api.rs:74-105`；
- `sync_legacy_mcp_connection`：`:109-151`；
- delete：`:223-228`。

创建失败补偿时会同时删 MCP server 和 IntegrationDefinition，但正常 delete 路径只明显删除 MCP server。即使其他层有清理，这种双写 API 仍要求所有更新/删除/恢复路径保持对称。

**建议**：选择 Connections/IntegrationDefinition 为唯一权威，legacy MCP endpoint 只做 adapter；或执行一次性迁移并下线 legacy CRUD。投影应可重建，不应靠每个 handler 记住补偿动作。

### P2-06 Goal objective 仍有 WorkForm 与 GoalRecord 两个持久表示

**证据**：`crates/opentopia-core/src/store/goal_event_repository.rs:124-133`

代码已明确 WorkForm authoritative，并在读 GoalSnapshot 时覆盖 legacy `GoalRecord.objective`，同时注释等待专门 migration 删除旧列。这是比 Turn 状态更健康的“读时派生”，但迁移尾巴仍存在。

**建议**：给该迁移明确 schema version 和删除期限；迁移后 API mapper 从 WorkForm 生成 objective，GoalRecord 持久模型不再包含字段。不要长期把“以后 migration”当兼容策略。

### P2-07 数据库索引列与 `document_json` 重复存储相同状态

**证据**：

- `crates/opentopia-core/src/store.rs:309-373`：Flow draft 的 revision/status/updated_at 与完整 JSON 同写；
- `crates/opentopia-core/src/store/sqlite_runtime.rs:61-69`：恢复时 SQL 同时修改列和 `json_set(document_json, ...)`；
- Flow run、trial、automation 等还有相同模式。

为了查询性能保留索引列是合理的，但当前 JSON domain object 也含这些字段，所有写路径必须双写同一事实。

**建议**：数据库列成为唯一 metadata authority，payload JSON 不再序列化 status/revision/timestamp；读模型组装 domain object。若短期不能迁移，至少集中 codec/update helper，并用 CHECK/round-trip 测试检测漂移。

### P2-08 canonical JSON 和 stable serializer 重复实现

**证据**：

- Rust：`crates/opentopia-core/src/agent.rs:2351` 与 `crates/opentopia-core/src/context_runtime.rs:568`；
- TypeScript：`apps/desktop/src/usageLogs.ts:715`；
- Evaluation：`evaluation/src/graders.mjs:352`。

它们用于 signature、hash、日志和评测，语义漂移会直接影响缓存命中、幂等或评测稳定性。

**建议**：Rust 只保留一个 versioned canonical JSON module；JS/TS 共用一份 package/module，并维护跨语言 golden vectors。

### P2-09 renderer 已有 `errorMessage`，仍有大量本地复制

**证据**：`apps/desktop/src/errorMessage.ts` 已存在统一函数，但 conversation controller、connections store、automation store、PreviewHost、SpreadsheetGrid、WebPreviewSurface 等至少十余处又定义同名函数，`App.tsx` 还有大量 inline ternary。

这不是重大 bug，但说明 feature 模块在重复建立微型基础设施。统一 helper 后还能集中处理 `ApiResponseError`、cause chain、用户可见与诊断信息的区别。

### P2-10 Windows verbatim path 规范化跨安全边界重复

**证据**：相同的 `\\?\`/`\\?\UNC\` 处理出现在：

- `crates/opentopia-core/src/enterprise.rs:272`
- `crates/opentopia-core/src/policy.rs:588`
- `crates/opentopia-core/src/sandbox/path_policy.rs:61`
- `crates/opentopia-core/src/execution_runtime.rs:210`
- `crates/opentopia-core/src/workspace_execution_capsule.rs:581`
- `crates/opentopia-server/src/terminal_api/runtime.rs:540`
- Windows sandbox 的 `env.rs:187`、`process_launch.rs:293`、`acl_persistence.rs:1109`。

不同调用点确实需要 display、process-native、comparison identity 等不同语义，因此不应简单合成一个 `normalize_path(String)`。应建立 typed path identity API，明确每个模式，并共享一套 drive/UNC/case/trailing separator/junction 测试矩阵。

### P2-11 design check 的扫描范围不足

**证据**：`scripts/check-design-system.mjs:6-7` 只扫描 `components/ui` 和 `styles/ui.css`。但实际 feature styles 中仍可见：

- `app-workspace.css:84, 92, 461` 的裸 `z-index`；
- `app-workspace.css:1020` 与 `app-settings.css:526-533` 的裸色值。

因此 `pnpm design:check` 通过不能证明整个新增 UI 遵循 token 规则。

**建议**：扫描所有本次改动的 desktop UI 文件；legacy 存量用带 owner 和到期日的 baseline allowlist，而不是缩小扫描目录。这样新代码不能继续增加债务，也无需一次性重写全部旧 CSS。

### P2-12 Evaluation `deepEqual` 对对象 key insertion order 敏感

**证据**：`evaluation/src/graders.mjs:29-30` 直接比较 `JSON.stringify(actual) === JSON.stringify(expected)`，同文件 `:352` 已经有会排序 key 的 `stableSerialize` 却未用于 equality。

语义相同、属性插入顺序不同的对象可能评测失败。应使用结构化 deep equality 或 canonical serializer，并增加 key-order 回归测试。

### P2-13 prompt runtime 同时保留 legacy 与 compact private builders

**证据**：`crates/opentopia-core/src/prompt_runtime.rs:403-493` 有多组 `#[allow(dead_code)]` legacy builder；随后 `:493+` 又有 compact 版本。注释称为旧 integration 兼容，但 private dead functions 本身不能成为外部兼容接口。

当前工作树正在重构 prompt 文件，这项应视为迁移中的债务，而不是稳定设计缺陷。重构完成后删除 dead builders；若真要兼容，定义版本化 prompt artifact/fixture，而不是保留不可调用函数。

### P2-14 无界队列在 semaphore 之前生成等待 task

**证据**：

- `crates/opentopia-server/src/agent_runs.rs:45` 使用 unbounded channel；
- `:93-99` 每个 command 立即 `tokio::spawn`，task 内才等待 global/session semaphore；
- `crates/opentopia-server/src/bootstrap.rs:207` 的 root turn queue 也无界。

并发执行有上限，但等待中的 command/task 没有背压。正常桌面负载下风险不高，不过 burst、重复唤醒或恢复风暴会把内存队列当隐式持久化。

**建议**：使用 bounded channel，或直接从 durable DB claim；拿到 global permit 后再 spawn 执行 task。加入 per-turn dedup key 和 queue depth telemetry。

## 6. 其他值得收尾的事项

### 6.1 Provider 未配置时静默使用 MockProvider

`crates/opentopia-core/src/provider.rs:1088-1129` 在普通 provider 和 guardian provider 未配置时都回落到 `MockProvider`；其响应位于 `:1552+`。这会让配置错误看起来像模型成功回复。

建议改为 `UnconfiguredProvider` 的 typed error；Mock 只在显式 demo/test mode 注册。first-run UX 属于 desktop/bootstrap，不属于 provider core。

### 6.2 构建脚本重复 runtime 发现、hash 和 manifest 校验

`scripts/build-desktop.ps1`、`prepare-office-runtime.ps1` 和 `prepare-agent-tools-runtime.ps1` 重复处理 target、manifest、hash、版本探测。应把“准备”“验证”“stage”拆成可组合命令，并让 build 只消费标准化 runtime descriptor。

### 6.3 工作树卫生正在干扰审查和自动化

审查快照有 149 个 status entry；根目录还有 4 个 `UserDataOllamaparts*.bin`，合计约 **920.8 MiB**，以及 `.codex-tmp*`、`.tmp-*`、probe 文件、两个嵌套 `.git` 应用。当前 `.gitignore` 没覆盖其中很多模式。

这不等于这些文件都应该删除；其中可能有用户数据或正在进行的实验。本报告不做删除。建议建立单一 `.artifacts/` 或 `.local-work/` 根目录，更新 ignore，并在 CI 检查意外大文件和嵌套仓库。

## 7. 不应仅凭行数“简化”的模块

以下区域复杂度有真实领域来源，错误的抽象会比大文件更糟：

1. **Windows sandbox/ACL/process launch**：Windows token、ACL、job object、verbatim path 和进程环境本就复杂；应统一 path identity 和安全 invariant，但不要为了少行数合并不同安全阶段。
2. **Provider protocol codecs**：OpenAI Chat、Responses、Anthropic、Codex app server 的语义确有差异；应共享 canonical model/fixtures，不应追求一个充满 feature flag 的万能 codec。
3. **Turn changes 和 git diff**：Git tree、journal、hunk 和恢复逻辑适合按明确子领域拆，但不能把事务顺序抽象掉。
4. **生成契约、schema、lockfile、迁移和测试 fixture**：体积不是维护复杂度的直接指标，优先减少生成源和手写影子类型，而不是压缩生成输出。
5. **大量测试代码**：测试行数不是坏事。应减少的是重复 fixture 构造和漏跑，而不是测试覆盖。

## 8. 推荐实施顺序

### Phase 0：安全与错误语义（1 个短周期）

1. 修复 `logs:read`，改为 log ID capability；
2. 删除 sibling `.env` 扫描和业务变量别名；
3. Provider 未配置返回 typed error，不再默认 Mock；
4. SSE `Lagged` 显式补放或重连；
5. Flow condition parse error 不再静默为 `false`。

验收：安全边界测试、配置来源测试、SSE gap 测试、condition compile tests 全部进入 `check:full`。

### Phase 1：协议统一（1-2 个周期）

1. 生成 endpoint-keyed HTTP client，移除任意 `get<T>/post<T>`；
2. 定义 IPC ChannelMap，生成 preload/global types 并统一 runtime validation；
3. 采用标准 JSON Schema validator；Flow condition 编译 AST；
4. 统一 canonical JSON 和 `ContentDigest`。

验收：手写影子类型数量显著下降；非法 request/response 在边界失败；hash 带算法版本；跨语言 golden fixtures 通过。

### Phase 2：状态单一权威（2-3 个周期）

1. AgentTurn 成为唯一执行生命周期；产品 Turn 改 read model；
2. Connections 成为 MCP 唯一权威；移除 legacy 双写；
3. 完成 WorkForm/GoalRecord objective migration；
4. 数据库 metadata 列与 JSON payload 去重。

验收：删除补偿写和读时修复分支；所有 projection 可重建；migration 有 schema version 和回滚/备份策略。

### Phase 3：持久化热路径（2-4 个周期）

1. Flow 改 durable scheduler + append-only checkpoint/write；
2. 增加 latest snapshot、get-by-id、aggregate counters 和 pagination；
3. conversation read model + renderer windowed cache；
4. bounded queue/durable claim。

验收：长线程和大 Flow 的延迟/内存基准；重启、CAS 冲突、SSE lag、队列 burst 故障注入；不再以完整历史/完整 JSON 大小线性放大常用操作。

### Phase 4：UI 与接口收敛（可与 Phase 2/3 并行）

1. App 按 session ownership 拆状态，不做纯 JSX 搬家；
2. batch thread activity 替代 N+1；
3. 按 feature 删除 legacy CSS selector，最终删除整个 legacy layer；
4. SessionStore 与 ToolInvocationContext 从 Flow/Agent 热路径开始缩窄；
5. 清理 dead prompt builders、本地 error helper 和重复 path normalization。

验收：App 不再拥有领域数据细节；RightPanel 不再有百级 props；CSS owner 唯一；新增工具只依赖声明能力；旧迁移层能按文件整体删除。

## 9. 建议的架构约束

后续每个改动可以用以下问题做 review gate：

1. 这个字段/状态的唯一权威在哪里？其他表示能否完全重建？
2. 这个边界是否有运行时验证，还是只有编译期类型和 `as T`？
3. 失败是显式状态，还是被 `unwrap_or_default`、`result.ok()`、`let _ =` 隐藏？
4. 每次操作的成本是否随完整历史、完整文档或全部线程线性增长？
5. 这是一次迁移，还是会永久保留的新旧双路径？迁移删除条件是什么？
6. 新抽象能删除哪些补偿、分支或重复实现？如果不能删除，是否只是增加了一层？
7. 复杂度来自真实领域，还是来自宽接口、共享可变状态和协议漂移？

## 10. 建议的验证矩阵

| 领域 | 必须覆盖 |
|---|---|
| Credential | 明确 source、无 sibling scan、未配置 typed error、测试/演示显式 Mock |
| IPC | request/response schema、sender authorization、path capability、malformed payload |
| HTTP | endpoint key 与 method/path、request/response runtime decode、生成物 drift |
| SSE | history/live 无缝衔接、Lagged、重连、sequence gap、慢消费者 |
| Flow | crash recovery、lease、重复提交、CAS 冲突、并行节点幂等、失败终态 |
| Turn | new/resume/cancel/preflight/restart 的单一状态机与投影重建 |
| Context | 10/1,000/100,000 events 下的查询和内存基准、数据库错误传播 |
| Hash | 算法版本、canonical JSON golden vectors、旧 digest 迁移、collision-sensitive 用途 |
| Desktop | renderer tests、Electron tests、design tokens、keyboard/accessibility、长会话内存 |
| Windows path | drive/UNC/verbatim/case/trailing separator/symlink-junction/path sibling |

## 11. 最终判断

OpenTopia 当前的主要问题不是“代码写得不够短”，而是**权威状态、协议、持久化和 UI ownership 的边界尚未完全收敛**。不少局部实现本身写得认真，也已经在做补偿、恢复、分阶段加载和兼容；恰恰因为根边界不稳定，这些正确的局部努力叠加成了整体复杂度。

最值得优先删除的不是某个长函数，而是：

- 第二份权威状态；
- 第二套协议解释；
- 第二层迁移覆盖；
- 每次完整历史扫描；
- 每次完整文档重写；
- 隐藏错误的默认值；
- 能接受任意类型/路径/能力的宽边界。

如果按上述顺序推进，代码量会自然下降，但更重要的是：新增功能不再需要同时修改多条平行路径，性能成本不再随历史线性增长，错误也会在正确边界被发现，而不是继续由下游补丁兜底。
