# OpenTopia 工具提示词与 Prompt Cache 调查

日期：2026-08-09

> 2026-08-10 实现更新：本文第 1～7 节保留修复前的实测基线；第 8～12 节描述的是修复前实现与当时建议。当前实现状态以紧随其后的“实现结果”小节为准。

## 0. 2026-08-10 实现结果

本次已经把工具面改成两层正交分类，而不是根据每条消息做复杂的逐工具意图门控：

- 工作体验模式：Work/Code 共享 Common；Flow 在 Common 上追加 `flow_*`。
- 协作流程模式：Plan 只追加 `request_user_input`；Goal 追加 `set_plan`、`update_plan`、`complete_task`。
- Multi-agent 属于 Common，但仅在运行时确实配置了 scheduler 且没有关闭多 Agent 时暴露。
- Bundled plugin 与 MCP 统一视为 External，完整 Schema 默认延迟。

Provider 降级矩阵如下：

| 能力层 | 初始可见内容 | 完整 Schema 何时出现 |
|---|---|---|
| OpenAI Responses + 已知支持 Tool Search | `DeferredNamespace` 默认只让模型看到 namespace 名称和简述；也支持 `DeferredIndividual` 的工具名称和简述 | Provider 执行 hosted `tool_search` 后追加 |
| 不支持 namespace、但支持 deferred/tool search | 单工具名称和简述 | Provider 执行 hosted `tool_search` 后追加 |
| Chat Completions、Anthropic、未知 Responses relay | Common 完整 Schema；External 用本地 `tool_search` 和精简工具组目录 | 本地搜索命中后的下一模型 Round 追加；若明确选择 Eager 则直接展平 |

当前用户消息仍是三种 Provider 初始消息序列的最后一项。Responses Tool Search continuation 的 call/output 与后续 function result 只能追加在它后面。工具仍通过 Provider 顶层协议传递，OpenTopia 不声称能够控制厂商内部 token 编译位置。

Usage 日志现在把原来的 `toolSchemas` 进一步拆成：

- `directToolSchemas`：初始直接加载的工具和输出 Schema；
- `deferredToolCatalog`：延迟工具的名称/简述或 namespace 目录；
- `loadedToolSchemas`：Tool Search continuation 实际追加的定义。

桌面发布脚本会强制运行 `scripts/test-provider-tool-cache-release.ps1`。门禁跨 OpenAI Responses、OpenAI-compatible Chat 和 Anthropic 三个适配层验证能力降级、初始用户消息位置、显式缓存 breakpoint 与 Tool Search 追加顺序。详见 `docs/provider-tool-cache-release-gate.md`。

## 1. 结论先行

1. **第一次模型请求已经知道本轮用户消息。** 服务端先收到用户的 `content`，再构建上下文和 `ModelRequest`。因此从技术上可以在第一轮请求前按用户当前意图筛选工具，不需要等模型请求完成一次。
2. **但当前 OpenTopia 没有按当前用户消息筛选 Core/Plugin 工具。** 当前逻辑遍历所有“符合运行时权限”的工具，并把每个工具的名称、完整描述和完整 JSON Schema 都装入 `toolCandidates`。
3. **工具不在 system prompt 文本内部。** 它们是 `ModelRequest.toolCandidates`，到 OpenAI Chat Completions 适配器后成为 HTTP 请求的顶层 `tools` 字段。
4. **在实际 HTTP JSON 中，`messages` 字段在前、`tools` 字段在后。** 但 JSON 字段顺序不能等同于模型服务内部的 prompt/token 排列顺序。OpenAI 没有公开“工具相对 system/developer/user 消息究竟排在模型前缀的哪个位置”。因此不能严谨地声称工具在模型内部一定靠前或靠后。
5. **可以确认工具参与缓存前缀匹配。** OpenAI 官方文档明确要求请求间的工具定义保持完全一致；Prompt Cache 只匹配精确前缀。改变工具名称、描述、Schema、数量或顺序，都可能让缓存从差异点开始失效。
6. **工具远不只是标题和简述。** 最新真实请求中有 38 个工具，工具区约 8.3k 本地估算 token：名称约 1.7%，描述约 24.1%，参数 Schema 约 66.8%，JSON 包装约 7.4%。
7. **工具在冷启动请求中占比非常高。** 两个真实首轮样本的工具区约占请求的 50%～53%；对话历史增长后，占比下降到约 8%～11%。
8. **建议不要每一模型 Round 都重新裁剪工具。** 更合理的边界是：收到用户消息后选择一次“本 User Turn 的稳定工具集”，同一 Turn 内保持不变；确需发现新工具时只追加、不删除、不重排，并记录 catalog hash。

## 2. 第一次请求是否知道用户意图

知道的是“用户当前发来的原始消息”，而不是模型分析后的隐含意图。

当前链路是：

```text
收到用户 content
  -> 构建上下文、技能与 Agent
  -> 计算 provider_tool_catalog()
  -> 创建 ModelRequest
  -> 首次调用模型
```

所以第一轮前可以做两类筛选：

- 规则型筛选：体验模式、附件类型、文件扩展名、显式关键词、当前权限、是否启用多 Agent、是否已有 Goal/Flow。
- 轻量语义路由：用小模型或本地分类器把当前消息映射到固定工具 Profile。

当前实现只完成了权限/运行时可用性筛选，没有用本轮用户消息给 Core/Plugin 工具做意图路由。`eligible_provider_tool_candidates()` 直接遍历注册表，把每个候选的完整定义送入模型。

用户需求后续会变化，这不意味着必须每个 Round 动态更换目录。建议区分两个边界：

- **新 User Turn：** 可以根据新用户消息重新选择工具 Profile。
- **同一 User Turn 的多个模型 Round：** 默认冻结工具集，避免反复破坏缓存；需要新能力时通过 `tool_search` 追加一次。

## 3. 工具到底在提示词的哪里

### 3.1 OpenTopia 逻辑请求

工具与 system prompt 平级，不属于 system prompt 字符串：

```json
{
  "systemPrompt": "...",
  "conversation": [],
  "userMessage": "...",
  "toolCandidates": [
    {
      "name": "read_file",
      "description": "...",
      "inputSchema": { "type": "object", "properties": {} }
    }
  ]
}
```

### 3.2 当前真实 OpenAI Chat Completions 请求

2026-08-08 最新记录的顶层字段顺序为：

```text
messages
model
parallel_tool_calls
prompt_cache_key
reasoning_effort
stream
stream_options
tool_choice
tools
```

请求形状是：

```json
{
  "messages": [
    { "role": "system", "content": "..." },
    { "role": "developer", "content": "..." },
    { "role": "user", "content": "..." }
  ],
  "model": "gpt-5.6-terra",
  "prompt_cache_key": "opentopia-11beee009cbf514a",
  "tool_choice": "auto",
  "tools": [
    {
      "type": "function",
      "function": {
        "name": "read_file",
        "description": "...",
        "parameters": { "type": "object", "properties": {} }
      }
    }
  ]
}
```

这里的“`tools` 在 HTTP JSON 最后”只是传输层事实。模型服务会把结构化请求编译成内部 token 序列，API 文档没有公开 `tools` 相对 `messages` 的内部排序。因此：

- 可以说“工具不在 OpenTopia 的 system prompt 里”；
- 可以说“HTTP body 中 tools 是最后一个顶层字段”；
- **不能**据此说“工具在模型缓存前缀里一定靠后”。

### 3.3 缓存层能确认什么

[OpenAI Prompt Caching 官方文档](https://developers.openai.com/api/docs/guides/prompt-caching)说明：

- 只有精确 prompt 前缀才能命中；
- 静态内容应放前面，变量内容应放后面；
- 该要求同样适用于工具，工具在请求间必须完全一致；
- `prompt_cache_key` 帮助路由，但不会让两个不同前缀强行命中。

因此你的担心是成立的：如果工具目录发生变化，而且 provider 把工具渲染在历史消息之前，那么差异后的对话历史也会失去前缀命中。即使工具被渲染在消息之后，工具自身的缓存也会失效。确切损失边界只能通过 provider 的 `cached_tokens` 和受控 A/B 实验判断，不能从 JSON 字段顺序推断。

另外，本次真实会话使用的是 OpenAI-compatible 自定义端点 `nowcoding.ai/v1/chat/completions`。OpenAI 官方规则是重要参照，但该服务的内部渲染与缓存实现不一定完全相同。

## 4. 真实 token 占比

### 4.1 最新冷启动样本

数据源：`.opentopia/opentopia.db`，线程 `7445edbc-7bee-4ccc-ab17-ede3622f95a1`，首个 Turn 首轮请求。

| 指标 | 数值 |
|---|---:|
| 工具数量 | 37 |
| 工具 Schema 本地估算 | 7,454 token |
| 整体逻辑输入本地估算 | 13,980 token |
| 工具占逻辑输入 | **53.32%** |
| HTTP `tools` 字段 | 30,146 bytes |
| 完整 HTTP body | 58,620 bytes |
| 工具占 HTTP body | **51.42%** |
| Provider 报告输入 | 10,171 token |
| Provider 报告缓存输入 | 0 token |

本地估算与 provider tokenizer 的绝对值有误差，但“工具约占一半”同时得到了逻辑 token 分类和 HTTP 字节比例两种口径的支持。

首轮本地输入分类：

| 分类 | 估算 token |
|---|---:|
| Base instructions | 3,534 |
| Developer instructions | 1,018 |
| Repository instructions | 205 |
| Runtime context | 846 |
| Skill instructions | 495 |
| Current user | 428 |
| **Tool schemas** | **7,454** |
| **Total** | **13,980** |

### 4.2 同一 Turn 的缓存表现

该 Turn 的 28 个模型 Round 中，工具目录始终是 37 个，目录 hash 始终为 `fd5ece8662`。

| Round | Provider input | Cached input | 缓存占比 | 工具估算 |
|---:|---:|---:|---:|---:|
| 1 | 10,171 | 0 | 0% | 7,454 |
| 2 | 10,306 | 9,728 | 94.39% | 7,454 |
| 5 | 13,169 | 12,800 | 97.20% | 7,454 |
| 10 | 27,787 | 27,136 | 97.66% | 7,454 |
| 28 | 79,716 | 78,336 | 98.27% | 7,454 |

这说明稳定目录在长 Turn 中能获得很高的缓存复用。Provider 只报告总 `cached_input_tokens`，没有按“工具/系统指令/历史”拆分，因此不能声称 7,454 个工具 token 全部精确命中；但结果与“稳定工具属于缓存前缀的一部分”一致。

### 4.3 最新 38 工具目录

后续激活 `spreadsheet` 后，目录变为 38 个，hash 为 `a8aba13f33`：

| 场景 | 工具估算 | 本地总输入估算 | 工具占比 |
|---|---:|---:|---:|
| 38 工具首次记录，本轮已有历史 | 8,293 | 41,845 | 19.82% |
| 更长对话的新 Turn 首轮 | 8,293 | 76,267 | 10.87% |
| 长 Turn 第 16 Round | 8,293 | 97,913 | 8.47% |

所以“工具占比到底多大”没有单一常数：

- 冷启动、历史很短：约 **50%～53%**；
- 中等历史：约 **20%**；
- 很长历史：约 **8%～11%**。

工具 token 仍会出现在每次逻辑请求中，但缓存命中部分按 provider 的 cached-input 规则计费，不能把名义 token 占比直接当成实际成本占比。

### 4.4 旧样本交叉验证

2026-08-05 的 39 工具首轮请求中：

- `tools`：27,153 bytes；
- 完整 body：52,401 bytes；
- 字节占比：51.82%；
- 本地粗估工具：约 6.8k token；
- Provider 输入：13,547 token。

旧版当时还没有记录 `input_breakdown.toolSchemas`，所以 6.8k 只能视为估算，但其约 50% 的结果与最新样本一致。

## 5. 为什么“标题 + 简述”也会变成 8.3k

最新 38 工具的 OpenAI Chat wire schema 估算分解如下：

| 内容 | 估算 token | 占工具区 |
|---|---:|---:|
| 工具名称 | 142 | 1.7% |
| 工具描述 | 2,018 | 24.1% |
| `parameters` JSON Schema | 5,602 | 66.8% |
| JSON 包装/字段名/分隔符 | 621 | 7.4% |
| **合计** | **8,383** | **100%** |

`8,383` 是实际 wire `tools` 的估算；事件中的 `8,293` 是 provider-neutral `toolCandidates` 的估算，两者结构略有不同。

因此：

- 如果真的只发送名称和一句简述，通常不会有 8k；
- 当前不是这种实现，而是把完整、可校验的参数 Schema 一起发送；
- 部分描述也不短，`update_plan`、`set_plan`、`create_skill` 等包含较长的行为约束；
- 复杂 Schema 会重复出现 `properties`、`required`、`type`、`description`、`default`、`minimum`、`maximum` 等字段。

最大的 10 个工具占整个工具区约 58.9%：

| 工具 | 估算 token | JSON bytes |
|---|---:|---:|
| `update_plan` | 1,085 | 4,340 |
| `spreadsheet` | 847 | 3,385 |
| `create_skill` | 433 | 1,730 |
| `set_plan` | 422 | 1,688 |
| `flow_create` | 400 | 1,598 |
| `flow_update` | 383 | 1,531 |
| `shell` | 366 | 1,461 |
| `read_files` | 345 | 1,380 |
| `search` | 334 | 1,333 |
| `read_file` | 321 | 1,284 |

## 6. 当前实际工具提示词示例

下面是最新记录中 `read_file` 的完整 wire 定义。可以看到它不是“标题 + 简述”，而是完整参数协议：

```json
{
  "function": {
    "description": "Read a UTF-8 text file inside the workspace. Use one-based startLine/endLine to read an exact source range, or offset/limit for a character window; the two modes are mutually exclusive. Returns at most 16000 characters per call and reports nextLine or nextOffset when more content remains.",
    "name": "read_file",
    "parameters": {
      "additionalProperties": false,
      "properties": {
        "endLine": {
          "default": null,
          "description": "Optional inclusive one-based final line. Requires startLine.",
          "format": "uint64",
          "minimum": 1.0,
          "type": ["integer", "null"]
        },
        "limit": {
          "default": null,
          "description": "Maximum characters to return, capped at 16000. Cannot be combined with startLine or endLine.",
          "format": "uint64",
          "maximum": 16000.0,
          "minimum": 1.0,
          "type": ["integer", "null"]
        },
        "offset": {
          "default": null,
          "description": "Character offset to start reading from. Defaults to 0 in character mode. Cannot be combined with startLine or endLine.",
          "format": "uint64",
          "minimum": 0.0,
          "type": ["integer", "null"]
        },
        "path": {
          "description": "File path relative to workspace.",
          "type": "string"
        },
        "startLine": {
          "default": null,
          "description": "One-based line number to start reading from. Enables line mode.",
          "format": "uint64",
          "minimum": 1.0,
          "type": ["integer", "null"]
        }
      },
      "required": ["path"],
      "type": "object"
    }
  },
  "type": "function"
}
```

## 7. 最新记录中的完整工具目录

以下为 2026-08-08 最新活动目录。Schema 会随体验模式、权限、插件、MCP、是否启用多 Agent 等条件变化，不代表每个线程永远固定为这 38 个。

| # | 工具 | 单工具 Schema 估算 token | JSON 字节 | 描述 |
|---:|---|---:|---:|---|
| 1 | `apply_patch` | 154 | 615 | Apply workspace edits by passing exactly one JSON field named `patch`. |
| 2 | `background_output` | 203 | 811 | Control background jobs and persistent stdio sessions. |
| 3 | `cancel_agent` | 64 | 254 | Cancel an active child agent. |
| 4 | `complete_task` | 227 | 905 | Finish the current user task after its requested scope has been verified. |
| 5 | `create_skill` | 433 | 1,730 | Create a reusable Skill directly from the current conversation. |
| 6 | `flow_cancel` | 68 | 269 | Request cancellation of a Flow run at the next node boundary. |
| 7 | `flow_create` | 400 | 1,598 | Create and bind a complete FlowDraft from a workflow or successful Run/Trace. |
| 8 | `flow_inspect` | 93 | 369 | Inspect a FlowDraft or immutable published Flow version. |
| 9 | `flow_pause` | 89 | 353 | Request a Flow run pause at the next node boundary. |
| 10 | `flow_publish` | 103 | 410 | Publish an immutable Flow version after validation and simulation. |
| 11 | `flow_resume` | 146 | 582 | Resume a paused Flow run or resolve its approval node. |
| 12 | `flow_run` | 114 | 456 | Start an immutable published Flow in the durable Flow Runtime. |
| 13 | `flow_search` | 64 | 256 | Search reusable published Flows and current Flow drafts. |
| 14 | `flow_simulate` | 115 | 460 | Compile and dry-run a valid FlowDraft. |
| 15 | `flow_status` | 84 | 335 | Inspect one durable Flow run or list recent runs. |
| 16 | `flow_update` | 383 | 1,531 | Replace a FlowDraft specification using optimistic revision control. |
| 17 | `flow_validate` | 93 | 369 | Validate graph topology, schemas, capabilities, risk gates and budgets. |
| 18 | `followup_task` | 123 | 489 | Give an existing agent a follow-up task. |
| 19 | `git_diff` | 43 | 171 | Show the current git diff for the workspace. |
| 20 | `interrupt_agent` | 92 | 367 | Interrupt an agent's current turn. |
| 21 | `list_agents` | 91 | 361 | List visible agents in the current root task tree. |
| 22 | `list_files` | 91 | 364 | List direct children of a workspace directory. |
| 23 | `list_skills` | 54 | 213 | List available Skills without loading instructions. |
| 24 | `read_attachment` | 177 | 706 | Read a user-attached text or document source. |
| 25 | `read_file` | 321 | 1,284 | Read a bounded UTF-8 text file or exact line range. |
| 26 | `read_files` | 345 | 1,380 | Read up to eight independent UTF-8 files concurrently. |
| 27 | `read_skill` | 178 | 711 | Read one selected Skill's instructions. |
| 28 | `search` | 334 | 1,333 | Recursively search workspace text with bounded context. |
| 29 | `send_input` | 87 | 345 | Send additional input to an active child agent. |
| 30 | `send_message` | 120 | 477 | Queue a message for a visible agent. |
| 31 | `set_plan` | 422 | 1,688 | Create or replace dependency-aware goal progress memory. |
| 32 | `shell` | 366 | 1,461 | Run a Windows PowerShell 5.1 workspace command. |
| 33 | `spawn_agent` | 271 | 1,081 | Create an independently running child agent. |
| 34 | `spreadsheet` | 847 | 3,385 | Inspect, read, create or update bounded XLSX workbooks. |
| 35 | `update_plan` | 1,085 | 4,340 | Apply an atomic mutation to external goal progress memory. |
| 36 | `view_attachment` | 180 | 719 | View a user-attached image. |
| 37 | `wait_agent` | 157 | 627 | Wait for one agent/mailbox activity. |
| 38 | `wait_agents` | 172 | 688 | Wait on several child agents. |

## 8. 当前实现为什么会全量加载

### 8.1 Core 工具注册表

`ToolRegistry::with_core_tools()` 注册普通 Core 工具后，又无条件加入 12 个 `flow_*` 工具。注册表使用 `BTreeMap`，`list()` 按名称排序。

### 8.2 Code / Work / Flow 都是不受限能力投影

`ExperienceSurfaceProfile::for_mode()` 当前对 Code、Work、Flow 三种模式都使用 `CapabilityProjection::unrestricted()`。因此 Code 模式并不会自然排除 Flow Schema。

### 8.3 Progressive disclosure 只延迟 MCP

当前渐进披露只有在以下条件成立时启用：

- 候选工具中存在 MCP 工具；并且
- Automatic 模式下候选数量超过 32，或显式使用 Progressive。

启用后只是把 MCP 工具换成一个 `tool_search`；Core 和 bundled plugin 仍然全部暴露。没有 MCP 时，即使有 38 个 Core/Plugin 工具，也不会启用 `tool_search`。

## 9. 工具组变化对缓存的具体风险

真实记录里，目录从 37 个变为 38 个时只新增了 `spreadsheet`，原有工具定义都没有变化。但由于当前目录按名称排序，`spreadsheet` 被插入到 `shell` 和 `update_plan` 之间，后面的工具位置全部后移。

```text
旧：... shell, update_plan, view_attachment, wait_agent, wait_agents
新：... shell, spreadsheet, update_plan, view_attachment, wait_agent, wait_agents
```

这说明即使只增加一个工具，也不是天然“在工具块末尾追加”。若 provider 对工具数组做顺序敏感的精确前缀缓存，差异会从插入点开始，工具块剩余部分以及内部渲染在它后面的内容都无法复用。

这次 37 -> 38 的两次记录间隔超过一小时，可能同时受到缓存过期影响，所以不能把下一次 cache miss 单独归因于新增 `spreadsheet`。但排序造成的前缀变化是确定事实。

## 10. 建议的优化顺序

### P0：先把观测口径固定下来

每次模型请求记录并展示：

- `tool_count`；
- `tool_catalog_hash`；
- `toolSchemas` 本地估算；
- Provider `input_tokens`、`cached_input_tokens`、`cache_write_tokens`；
- 与上一轮相比是新增、删除、Schema 改变还是顺序改变。

当前事件已经有 `input_breakdown.toolSchemas`，但 provider 不会给出“工具中有多少 token 命中缓存”的精确分类。UI/报告必须明确区分本地分类估算和 provider 权威总量。

### P1：按体验模式建立稳定 Profile

Code/Work 默认不应加载全部 12 个 Flow 工具。最新目录中：

| 工具组 | 工具数 | 估算 token | 占整个工具目录 |
|---|---:|---:|---:|
| Flow | 12 | 1,752 | 20.9% |
| Multi-agent | 9 | 1,177 | 14.0% |
| Plan/Goal | 3 | 1,734 | 20.7% |

仅在非 Flow 任务中移除 Flow 组，冷启动总输入预计就能减少约 12%～13%。`spreadsheet` 单工具再占约 847 token，应只在 XLSX 任务或已激活相应能力时暴露。

### P2：把“每 Round 动态工具集”改成“每 User Turn 稳定工具集”

建议规则：

1. 收到用户消息后，先用确定性规则选择固定 Profile；
2. 第一次模型请求就携带这个 Profile 的完整 Schema；
3. 同一 User Turn 的多个 Round 不删除、不重排工具；
4. `tool_search` 发现新工具时只追加到一个独立的 reveal 区，并保持发现顺序；
5. 到下一 User Turn 才重新评估 Profile。

这能兼顾“需求会变化”和“Round 内缓存稳定”。

### P3：把 progressive disclosure 扩展到 Core/Plugin

当前 `tool_search` 只解决 MCP 数量过多的问题。应支持：

- Core group：Flow、Plan、Multi-agent、Attachment、Workspace I/O；
- Bundled plugin group：Spreadsheet 等；
- MCP group。

首轮保留一个小而稳定的核心工具集及 `tool_search`。搜索结果可以返回名称和短描述；只有被选中的工具才从下一 Round 开始追加完整 Schema。

质量保护：高频、基础、容易被首轮立即调用的工具仍应预加载，例如 `read_file`、`search`、`shell`、`apply_patch`。不能为了省 token 把所有工具都变成二次发现。

### P4：避免按全局字母序重排动态目录

全局 `BTreeMap` 适合确定性，但不适合 append-only 缓存。建议请求层使用分段顺序：

```text
stable_core_profile
stable_mode_profile
revealed_tools_in_discovery_order
```

新 reveal 只能追加到最后一段。不要因为新工具名称的字母顺序，把已经发送过的工具移动到新位置。

### P5：最后再压缩单个 Schema

单 Schema 压缩应从大户开始：`update_plan`、`spreadsheet`、`set_plan`、`create_skill`、`flow_create`。

可评估：

- 删除模型不需要的 `default: null`、`format` 等冗余元数据；
- 缩短重复描述，把跨字段约束放到一个位置；
- 将低频复杂操作拆成可延迟加载的独立工具；
- 在 provider 支持且不损害严格校验时复用 schema 定义。

不过最大收益仍来自“不加载当前任务无关的完整 Schema”，而不是缩短工具名。

## 11. 与缓存相关的最终判断

你的两点直觉需要这样修正：

- **“一直改工具组会影响前缀命中”——是。** 工具必须完全一致才能获得相同缓存前缀，当前按字母排序还会把一次新增扩大成工具数组中段之后的变化。
- **“工具应该只有标题和简述，token 不多”——不符合当前实现。** 当前发送的是 38 个工具的完整 Schema，冷启动时约占整个输入的一半。

最稳妥的设计不是“工具永远全量固定”或“每轮都重新筛选”，而是：

> **首轮基于当前用户消息选一个小而稳定的 Turn 级工具 Profile；Turn 内冻结；缺能力时用 tool_search 做 append-only reveal；下一 User Turn 再重新路由。**

## 12. 代码证据索引

- `crates/opentopia-core/src/provider.rs:58-90`：`ModelRequest` 中 system prompt 与 `tool_candidates` 是独立字段。
- `crates/opentopia-core/src/provider.rs:127-134`：本地 token breakdown 单独统计 `tool_schemas`。
- `crates/opentopia-core/src/provider.rs:1296-1348`：Chat Completions body 构建并写入顶层 `tools`。
- `crates/opentopia-core/src/provider.rs:3966-3994`：每个 OpenAI tool 包含 name、description、parameters。
- `crates/opentopia-core/src/agent.rs:3211-3238`：候选工具装入完整 description 与 schema。
- `crates/opentopia-core/src/agent.rs:3241-3286`：渐进披露当前只在存在 MCP 工具时生效。
- `crates/opentopia-core/src/tools.rs:554-590`：Core 注册表包含 12 个 Flow 工具。
- `crates/opentopia-core/src/tools.rs:641-643`：工具由有序 Map 的 key 顺序列出。
- `crates/opentopia-core/src/enterprise.rs:188-210`：Code、Work、Flow 当前均为 unrestricted capability projection。
- `crates/opentopia-server/src/main.rs:6666-6673`：服务端把完整 provider tool catalog 纳入上下文预留估算。
- `crates/opentopia-server/src/main.rs:8077-8092`：自动 `prompt_cache_key` 当前不包含 tool catalog hash。

## 13. 测量限制

- Provider 的 `input_tokens` 与 `cached_input_tokens` 是权威总量，但不按 harness 模块分类。
- `toolSchemas` 是 OpenTopia 的本地估算，算法对 ASCII 约按 4 字符/token 计算；不能当作 provider tokenizer 的精确值。
- HTTP 字节占比是精确的传输层比例，但不是精确 token 比例。
- 本报告没有读取或输出用户消息正文，只分析了结构、工具定义和 usage 数字。
