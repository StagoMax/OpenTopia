# OpenTopia Harness 分层上下文实现

日期：2026-07-19

本文记录 OpenTopia 为接近 Codex 桌面端 harness 形态而完成的三阶段改造。目标不是复制某一版内部提示词，而是建立可分层、可追踪、可缓存、可迁移 provider 的上下文协议。

## 1. 结构化模型上下文

核心类型位于 `crates/opentopia-core/src/model_context.rs`：

- `ModelContextItem`：记录 kind、role、source、content、hash、token estimate、cache scope 和 sensitivity。
- `CompiledModelContext`：按稳定顺序渲染 system/developer 指令，并生成上下文 hash。
- `ThreadContextSnapshot`：冻结任务首次运行时的 provider、模型、工作区、体验模式、权限、沙箱、规则和工具目录 hash。
- `TurnContextSnapshot`：记录每轮 world state、与上一轮相比发生变化的 key，以及本轮上下文 hash。

模型可见层按以下顺序编译：

1. OpenTopia 基础 agent 指令。
2. 工作区、权限、沙箱和体验模式。
3. 用户级、工作区级和嵌套目录的 `AGENTS.md`。
4. 可缓存的 Skill 目录摘要。
5. 本轮选中的 Skill 正文。
6. 动态 world state，包括 Git、日期、平台以及 Skill/工具数量；目录正文不在每轮状态里重复。
7. durable summary、对话历史和当前用户输入。
8. 工具调用、工具结果和 provider 原生 response items。

`AGENTS.md` 解析位于 `crates/opentopia-core/src/instructions.rs`。解析顺序是：

- `~/.codex/AGENTS.md`
- `~/.opentopia/AGENTS.md`
- 工作区根目录到当前目录的 `AGENTS.md`

同一目录存在 `AGENTS.override.md` 时，它取代该目录的 `AGENTS.md`。读取有文件数、单文件大小、总大小和符号链接限制。

## 2. World State 与持久化事件

每个真实任务会持久化以下事件：

- `thread_context_snapshot`
- `turn_context_snapshot`
- `model_context_built`
- `model_request`
- `provider_request_sent`
- `provider_request_retried`
- `provider_response_received`

每轮模型调用生成一个 `request_id`。结构化上下文、逻辑请求、HTTP 请求、兼容重试和最终响应通过这个 ID 关联。自动上下文压缩使用 `round: 0`，表示 harness 内部模型请求。

模型请求和传输预览会脱敏：

- 不记录 Authorization header 或 API key。
- password、secret、access token 等已知字段替换为 `[REDACTED]`。
- 图片字节和 data URL 替换为长度摘要。
- 超大观测字符串有独立上限。

上下文压缩不会把上述观测事件再次送给摘要模型，避免递归膨胀。压缩后的消息和工具历史仍会物化为 `summary`、`conversation` 和 `tool_result` context items。

Skill 目录使用 `cacheScope: thread`，并排在所有 `turn` 层之前；动态 world state 使用紧凑 JSON 和 `cacheScope: turn`。完整 `WorldStateSnapshot` 仍写入事件用于审计和 changed-key 比较，但 provider 不再在每轮动态消息中重复 Skill 描述、目录 hash 等仅供 harness 使用的字段。

## 3. Provider Adapter 与 Responses API

`ModelProvider` 现在区分两个步骤：

1. `prepare`：把 provider 无关的 `ModelRequest` 转换成 `PreparedProviderRequest`。
2. `stream_prepared`：执行准备好的请求，并报告重试和响应观测事件。

支持的 provider 类型：

- `openai_compatible`：使用 `/chat/completions`，从 `contextItems` 生成原生 `system`、`developer` 和 `user` 消息。遇到 HTTP 400 时，兼容重试会把 developer 层合并回旧的单 system prompt，并在需要时压平 tool history。
- `openai_responses`：使用 `/responses`；基础 system 层进入 top-level instructions，developer 层成为 typed input message，用户输入保持 user message，函数工具和 SSE 事件继续使用 Responses 原生结构。
- `mock`：用于本地开发与测试。

逻辑 `ModelRequest.systemPrompt` 继续保存完整合并文本，作为旧 provider 的兼容面和可审计快照；它不再决定支持结构化角色的 provider 的实际消息角色。

Responses adapter 默认 `store: false`，并手动回放 provider 返回的 output items。reasoning、function call 和 function call output 不会被压成普通消息；它们按 `call_id` 关联。开启 reasoning 且保持 stateless 时，请求 encrypted reasoning content 并在下一轮回放。

Provider 设置新增：

- `storeResponses`
- `parallelToolCalls`
- `promptCacheKey`

未显式配置 cache key 时，OpenTopia 使用 provider、模型、工作区和体验模式生成稳定 key。旧设置缺少这些字段时自动使用隐私优先的默认值。

实现依据：

- [迁移到 Responses API](https://developers.openai.com/api/docs/guides/migrate-to-responses)
- [Message roles and instruction following](https://developers.openai.com/api/docs/guides/text#message-roles-and-instruction-following)
- [Function calling streaming](https://developers.openai.com/api/docs/guides/function-calling#streaming)

## 4. 桌面端检查方式

任务活动时间线会分别显示：

- `Thread context`
- `Turn world state`
- `Model context #N`
- `Model request #N`
- `Provider request #N`
- `Provider retry #N`
- `Provider response #N`

这些条目都可以展开查看 JSON。`Model request` 表示逻辑请求；`Provider request` 表示实际 adapter 生成的脱敏传输 body。二者应当分开检查。

## 5. 回归要求

至少验证：

- 旧 provider 设置仍能反序列化。
- Chat Completions 保持 system → developer → history → current user → tool history 的顺序，并验证 400 降级请求不再包含 developer role。
- Responses 把基础指令与 developer input 分开，且不会把 context 中的 user item重复发送。
- Skill 目录是 thread-cache item，动态 world state 不包含 Skill 长描述。
- Responses function schema 使用内部 tagged 结构。
- typed SSE text、function arguments、usage 和 output items 能完整聚合。
- `AGENTS.md` 顺序、override 和大小边界正确。
- 同一模型轮次的所有观测事件使用相同 `request_id`。
- 桌面端 TypeScript 类型检查和生产构建通过。
