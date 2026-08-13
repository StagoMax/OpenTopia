# OpenTopia 多 Agent 可见实测套件

这组用例通过真实 OpenTopia Server 执行，并用产品的线程、事件和子 Agent
存储链路记录运行过程。使用 `-VisibleInDesktop` 时，每个用例都会作为独立任务
出现在 Topia 桌面应用中，可查看子 Agent 状态、工具轨迹和最终汇总。

## 场景

1. `MA-VISIBLE-001`：并行创建三个只读 Agent，分别提取事实、约束和风险，再由
   根 Agent 汇总。
2. `MA-VISIBLE-002`：审阅 Agent 显式向分析 Agent 发送消息，验证 Agent 间消息
   协作和等待链路。
3. `MA-VISIBLE-003`：复用已完成的同一 Agent 执行后续复核，验证
   `followup_task` 的身份与上下文连续性。

套件的 Grader 会检查子 Agent 完成数量、无遗留运行、必要编排工具和最终完成
声明。Fixture 只包含静态文本，提示词明确禁止修改工作区，因此测试不会改动产品
源码。

## 可见运行

```powershell
.\scripts\evaluate-opentopia-tool-suite.ps1 `
  -EnvFile <provider-env-file> `
  -Profile <provider-prefix> `
  -SuitePath evaluation\examples\opentopia-multi-agent-suite\suite.json `
  -ProviderId <desktop-provider-id> `
  -VisibleInDesktop
```

`-VisibleInDesktop` 复用 `.opentopia/opentopia.db`，所以线程会出现在桌面应用。
入口会把选中 Provider 的凭据注入该 Provider 自己的 `apiKeySource`，并保留
`target.json` 中的“多 Agent 实测”标题前缀。
