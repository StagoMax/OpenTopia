# Plan 模式运行时设计

## 产品语义

Plan 是协作模式，不是执行计划状态，也不是 Goal 的草稿。它复用普通 Agent Runtime 做只读调查、结构化澄清和方案收敛，最终交付一个 `proposed_plan` 消息部件。Default / Goal 下的 `update_plan`通过完整快照维护实施期间的 `WorkForm` 清单；两者没有共享状态，也不会互相转换。

为避免对话中途切换模式时改变 provider tool catalog，根 Agent 在 Default、Plan、Goal 中看到同一组核心 schema。可调用性由运行时按当前模式校验：

- `request_user_input`只有 Plan 可执行；Default / Goal 调用会失败；
- `update_plan`只有 Default / Goal 可执行；Plan 调用会以“execution checklist tool”错误失败；
- 子 Agent 仍看不到结构化提问和共享 WorkForm 工具。

因此“schema 稳定”不等于“权限放宽”。模式提示告诉模型该用什么，工具执行入口是最终的确定性边界。

## 生命周期

```text
用户以 Plan 发送消息
  -> Turn 级 collaboration instruction 追加在当前用户 cache anchor 之后
  -> 默认 Runtime 做非变更型调查
  -> 可选 request_user_input
       -> 用户选择 / 自定义答案 / 跳过
       -> 同一个 Turn 继续
  -> 模型输出 <proposed_plan>...</proposed_plan>
  -> 流解析器移除标签，只流式展示正文
  -> 最终消息保存 ProposedPlan part
  -> Desktop 用独立方案卡渲染
  -> 用户点击“开始实施”
  -> 客户端以 Default 模式发送新的授权消息
  -> Default 可按复杂度新建自己的 WorkForm 执行清单
```

Plan 不创建 `GoalRecord`，不调用 `update_plan`，也不产生 `WorkFormUpdated`。如果模型没有输出 `<proposed_plan>`，响应仍按普通文本消息保存，适合继续澄清而不是发布不完整方案。

## 缓存边界

协作模式提示属于 Turn 动态尾部。OpenAI Responses 和 Chat 编码都把它放在当前用户消息之后，因此切换模式不会改写已经发送的历史前缀。Default 与 Plan 的根 Agent 工具 schema 保持一致，避免工具目录成为额外断点；运行时模式门禁独立于 schema 暴露。

仍会使缓存边界变化的因素包括模型/Provider 切换、外部工具或插件目录变化、权限/能力投影变化、上下文压缩 epoch 变化，以及基础提示版本变化。

## 与 Default `update_plan` 的边界

| 概念 | Plan `proposed_plan` | Default / Goal `update_plan` |
|---|---|---|
| 用途 | 交给用户审阅的完整实施方案 | Agent 实施期间的外部进度记忆 |
| 数据载体 | `MessagePart::ProposedPlan` | `WorkForm` + `WorkFormUpdated` |
| 是否可执行工具 | 否 | 是 |
| 是否有 revision / item status | 否 | 是 |
| 是否进入完成守卫 | 否 | 是 |
| 是否能在 Plan 模式修改 | 由新 `<proposed_plan>`完整替换 | 不能；运行时拒绝 |

这个边界刻意不提供“把方案卡直接写入 WorkForm”的隐式转换。“开始实施”只负责切换模式并追加一条新的 Default 用户消息；进入 Default 后，模型可以根据用户授权和任务复杂度新建自己的执行清单。执行清单是实施状态，不是对方案文本的第二份持久副本。
