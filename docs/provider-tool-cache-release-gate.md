# Provider 工具分层与缓存顺序发布门禁

桌面发布必须先运行 `scripts/test-provider-tool-cache-release.ps1`。`scripts/build-desktop.ps1` 已将它作为不可跳过的前置步骤。

门禁覆盖三个不同的 Provider 适配层：

| Provider 协议 | 工具能力层 | 预期降级 |
| --- | --- | --- |
| OpenAI Responses（支持 Tool Search） | `DeferredNamespace` / `DeferredIndividual` | namespace 或 function 带 `defer_loading`，并提供一个 hosted `tool_search` |
| OpenAI Chat Completions / compatible | 普通 function tools | 展平为完整 function schema，不发送 Responses 专属字段 |
| Anthropic Messages | 普通 Anthropic tools | 展平为完整 `input_schema`，不发送 Responses 专属字段 |

缓存顺序契约同时验证：

1. 初始请求中，Chat、Responses、Anthropic 的当前用户消息都位于各自消息序列末尾。
2. Responses 显式缓存时，稳定仓库指令、继承历史和当前用户消息的 breakpoint 位于预期边界。
3. Tool Search continuation 只能把 `tool_search_call`、`tool_search_output`、function call/result 追加在当前用户消息之后。

这些测试验证 OpenTopia 可控制的线协议顺序。Provider 如何把 HTTP `tools` 与消息最终编译成内部 token 序列属于厂商实现，不能从客户端精确断言；线上发布观测应另外比较相同长前缀的 `cached_tokens` / cache-write usage，但不能用该指标替代本门禁。
