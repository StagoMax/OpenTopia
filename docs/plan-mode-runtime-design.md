# Plan 模式运行时设计

## 产品语义

Plan 不是 Goal 的草稿状态，也不是一套单独的执行内核。它是在默认 Agent Runtime 上增加一个行为提示和一个可选工具：`request_user_input`。

模型可以直接调查并输出计划；只有需求、架构、技术选型、范围或风险存在会实质改变方案的歧义时，才调用结构化提问。一次调用包含 1～3 个问题，每题 2～3 个互斥选项，推荐项在前，并允许用户填写自定义答案或跳过。

## 生命周期

```text
用户以 Plan 发送消息
  -> 默认 Runtime 调查（shell/search/browser/multi-agent 按实际能力可用）
  -> 可选 request_user_input
       -> 用户选项 / 自定义答案 / 跳过
       -> 同一个 Turn 继续
  -> 普通 assistant message 输出完整计划
  -> Turn 完成后 Composer 回到 Default
```

Plan 不创建 `GoalRecord`，不调用 `set_plan` / `update_plan`，也不生成 `ProposedPlanCard`。计划正文就是普通消息，因此复用既有流式输出、历史、复制、搜索和缓存链路。

Plan 行为提示禁止在该 Turn 内实施或修改工作区，但不会删掉默认 Runtime 的系统工具。调查型 shell 命令和子 Agent 可以使用；根 Agent 负责向用户提问。真正的执行发生在 Plan Turn 完成后，由 Default 模式的新 Turn 承担。

## 与 Goal 的边界

Goal 是持久流程模式：由服务端创建 Goal，拥有 DAG、修订号、证据和完成约束，并额外暴露 `set_plan`、`update_plan`、`complete_task`。Plan 与 Goal 共享底层 Agent Runtime，但不共享持久流程状态。

## UI

`PlanChoiceCard` 只是 `request_user_input` 的对话内交互面板：推荐选项、自定义输入、继续与跳过。它不是最终计划的渲染组件，也不会把普通计划消息转换成特殊实体。
