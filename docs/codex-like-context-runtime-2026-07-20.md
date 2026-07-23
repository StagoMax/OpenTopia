# OpenTopia Codex-like Prompt 与上下文运行时

日期：2026-07-20

本轮改造的目标不是复制某个版本的 Codex 内部提示词，而是把相同类型的 harness 能力落实为可测试协议：稳定的 agent contract、分层上下文、可递归压缩的本地历史、Provider Prompt Cache、Responses 原生 compaction，以及可恢复的 turn 队列。

## 1. 基础 Prompt

基础 Prompt 位于 `crates/opentopia-core/src/base_agent_prompt.md`，由 `agent.rs` 使用 `include_str!` 编译进程序。当前版本为 `2026-07-20.1`，`ModelContextItem.metadata` 同时记录 `promptVersion` 和 `promptHash`。

基础 Prompt 只包含跨任务稳定的 agent contract：

- 请求类型和完成条件；
- 指令层级、权限和不可信观察数据边界；
- 工作区、已有改动和 Git 安全；
- Skill 触发与读取规则；
- 工具循环、子 Agent、验证和用户沟通。

工作区路径、权限模式、日期、Git 状态、AGENTS.md、Skill 正文和工具目录不写入基础 Prompt，而是在运行时进入对应的 thread、turn 或 round 层。

## 2. 每次模型请求的上下文顺序

`CompiledModelContext` 对所有 item 使用稳定顺序：

1. `stable`：基础 Prompt；
2. `thread`：工作区边界、权限、体验模式、AGENTS.md、Skill 目录；
3. `turn`：选中的 Skill、world state、durable summary、历史和当前输入；
4. `round`：本轮工具调用、工具结果和 Provider response items；
5. `none`：明确不参与复用的临时内容。

同一 cache scope 内保留插入顺序。指令渲染、context hash、token estimate 和 Provider 实际请求都使用同一排序结果，避免“观测快照和真实请求顺序不同”。

## 3. 本地历史与摘要

OpenTopia 仍由本地 SQLite 管理 Thread 真相源，而不是把长期会话状态只交给模型厂商：

- user/assistant 消息持久化；
- tool call/tool result 以 typed `MessagePart` 持久化；
- task plan、context snapshot、Provider request/response 和 usage 以事件持久化；
- 工具历史在下一轮以低权限、不可信 observation 重放，不提升为 System 指令；
- 自动摘要输入为 `previous durable summary + cursor 之后的连续消息和事件`；
- `coveredMessageCount` 和 `coveredThroughSeq` 只在实际覆盖后推进，单次 96K 字符上限不会让更老历史永久丢失；
- queued 消息只有轮到它开始执行时才进入历史，后排消息不会提前污染前一轮。

Token 估算统一采用 Unicode-aware 保守算法。实际历史预算会预留基础/开发者指令、AGENTS.md、Skill、world state、工具 schema、当前附件、模型输出和 reasoning 空间。桌面 Context meter 至少采用最近一次真实 `model_context_built` 估算，不再只统计聊天文本。

## 4. Prompt Cache 与原生 compaction

OpenAI Responses Provider 可配置：

- `promptCacheKey`：稳定路由 key；未配置时按 provider、model、workspace 和 experience mode 生成；
- `promptCachePolicy = explicit_30m`：发送 `prompt_cache_options`，并在最后一个可复用的 stable/thread developer item 上设置显式 breakpoint；
- `legacy_in_memory` 或 `legacy_24h`：发送旧模型使用的 retention；
- `responsesCompactionThresholdTokens`：发送 Responses `context_management` compaction，值必须至少为 4096 且小于 context window。

Provider usage 中的 cached input、cache write 和 reasoning tokens 会进入 typed event。桌面时间线显示单次指标，Context 面板显示 Thread 聚合值、compaction 次数和 context warning 数量。

自动本地摘要失败是非致命 `context_warning`，不会再被桌面误判为 turn 已终止。

## 5. 单窗口多轮实际怎么发送

OpenTopia 现在支持两种传输模式，但 SQLite 始终是应用侧真相源：

1. 默认 `storeResponses = false`：每次由本地历史重建完整有效上下文；稳定前缀交给 Prompt Cache 复用计算；
2. OpenAI Responses 且 `storeResponses = true`：保存成功响应的 `response_id`，下一轮发送 `previous_response_id + 当前输入`；
3. 顶层 `instructions` 仍按 Responses 协议随每次请求发送，已经进入远端 response 链的 developer 消息和对话历史不再重复发送；
4. 游标按 `(thread_id, agent_path)` 隔离，并绑定 provider、model 和 compatibility hash；hash 覆盖模型上下文、durable summary、Agent profile 指令与工具 schema；
5. 旧游标在 turn 开始前以事务方式取出并删除。失败、取消或进程中断不会重复使用一个状态未知的远端游标；
6. Provider 对游标返回 400/404 且错误指向 `previous_response_id` 时，同一次调用自动去掉游标并从本地完整逻辑上下文重放一次；
7. Provider 切换、model 切换或 compatibility hash 变化时直接忽略旧游标并完整重放。

客户端不探测厂商是否还保留 KV Cache。这个信息通常不可观测，也不应成为正确性依赖：状态 ID 决定“如何表达会话连续性”，Prompt Cache/KV Cache 只决定“Provider 是否能减少计算”。即使缓存和远端 response 都失效，本地历史仍可恢复请求。

## 6. 父 Agent 与子 Agent 分支

父 Agent 执行 `spawn_agent` 时，会冻结当时已经可见的 conversation 与 `CompiledModelContext`，而不是等子 Agent 真正排队运行时再读取父线程。`fork_turns = none | all | N` 在冻结快照上截取完整用户轮次。

子 Agent 的 profile developer 指令放在继承前缀之后、子任务输入之前。多个兄弟 Agent 因而具有字节一致的父前缀，Prompt Cache breakpoint 标记在继承历史末尾，可复用同一前缀计算；每个子 Agent 完成首轮后，再用自己的 `(thread_id, agent_path)` response 游标继续后续轮次。

这里不直接把父 Agent 的 `response_id` 当作所有子 Agent 的游标。父响应可能仍包含等待回填的工具调用，直接续接会把父工具协议状态泄漏到子分支。共享父前缀使用 Prompt Cache，分支内部续接使用各自的 `previous_response_id`，两层机制职责分开。

## 7. Turn 队列与恢复边界

活动 turn 期间提交的新消息不再返回 409，而是写入 SQLite `turn_queue`。当前 turn 结束后，独立 worker 按顺序启动下一条；未解决审批会阻止队列越过权限边界；服务重启会重新扫描并消费未处理队列。

正在执行的外部副作用不会在进程重启后盲目自动重放。已完成的 typed 工具记录、计划、消息和代码状态构成安全 checkpoint；被中断的任务可以在新 turn 中基于这些记录继续，避免重复执行未知状态的 shell、网络或写入操作。

## 8. 验证入口

- `scripts/check.ps1`：Rust workspace check、Desktop TypeScript 和 Vite production build；
- Server tests：预算、连续摘要 cursor、thread snapshot 签名、queued history 隔离和 typed tool replay；
- Core tests：Prompt contract、cache scope 顺序、Responses stateful/fallback/cache JSON、游标持久化、冻结分支截取、usage 解析、工具压缩安全和 SQLite 队列。

相关官方协议：

- Prompt caching: https://developers.openai.com/api/docs/guides/prompt-caching
- Conversation state: https://developers.openai.com/api/docs/guides/conversation-state
- Compaction: https://developers.openai.com/api/docs/guides/compaction
