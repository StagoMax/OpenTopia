# GLM-5.2 长程任务闭环评测（2026-07-16）

> 历史说明：本报告中的 8/24 轮执行预算和预算 continuation 是 2026-07-16
> 评测时的实现记录。2026-07-18 起，产品运行时已改为无总轮次/总时长上限的
> 进展式循环防护，并在上下文接近窗口边界时自动压缩工具历史，不再请求用户批准续跑。

## 结论摘要

OpenTopia 已在三类两阶段编码任务的最新有效运行中全部完成闭环：

- 账本解析与金额核对：`6/6 -> 8/8`。
- 配置迁移与稳定序列化：`6/6 -> 8/8`。
- 依赖图、发布波次与环检测：`6/6 -> 8/8`。

三项运行都通过了 Server 重启、SQLite 会话与计划恢复、公开测试、外部隐藏评分、
受保护文件检查、过程契约和密钥扫描。最新三项运行均没有 Turn 超时。

这证明当前实现具备“计划、实现、验证、阶段结束、重启恢复、继续实现、再次验证、
终态提交”的基础闭环能力。它不代表正式成功率：每个任务在最终版本上只有 1 次有效
运行，尚未达到每任务至少 3 次和至少 10 个长程任务的统计要求。

## 评测边界

本评测由 OpenTopia 本地 Harness 执行，不是 SWE-bench、Terminal-Bench 或其他公开
排行榜的官方成绩。方法上借鉴了以下原则：

- 初始 Fixture 必须失败，避免无效任务。
- Agent 只能访问公开规格和公开测试，隐藏 Grader 位于工作区外。
- 最终状态和过程轨迹分开评分。
- 超时、恢复失败、密钥泄漏或任务未闭环不能由部分分数抵消。

Provider 使用 `AUDIT_COPILOT_LLM` 配置和 `glm-5.2`。API Key 仅从
`J:\Project\信贷审核助手\.env` 注入评测进程，报告只记录 `redacted:set`，所有运行
均执行逐字节密钥扫描。

## 任务设计

每个任务包含两个阶段：

1. 阶段 1 实现核心库，运行公开测试，并通过显式计划更新结束当前范围。
2. Harness 停止并重启 OpenTopia Server，继续使用同一个 SQLite 数据库和工作区。
3. 阶段 2 恢复消息、事件、计划和代码差异，实现 CLI，再次运行测试并结束任务。
4. 外部 Grader 检查库和 CLI 的隐藏契约；Harness 同时检查计划、工具轨迹、恢复状态、
   受保护文件和密钥。

任务清单位于：

- `scripts/fixtures/long-horizon/task.json`
- `scripts/fixtures/long-horizon/config-migration/task.json`
- `scripts/fixtures/long-horizon/dependency-planner/task.json`

## 最新结果

| 任务 | 运行 ID | 阶段 1 | 最终 | Turn | 恢复 | 过程契约 | 耗时 | Token |
| --- | --- | ---: | ---: | --- | --- | --- | ---: | ---: |
| `LONG-LEDGER-001` | `glm-5-2-long-ledger-001-20260716T115515Z` | 6/6 | 8/8 | 2/2 成功 | 通过 | 通过 | 387,145 ms | 300,951 |
| `LONG-CONFIG-001` | `glm-5-2-long-config-001-20260716T123525Z` | 6/6 | 8/8 | 2/2 成功 | 通过 | 通过 | 389,493 ms | 178,526 |
| `LONG-DEPS-001` | `glm-5-2-long-deps-001-20260716T124156Z` | 6/6 | 8/8 | 2/2 成功 | 通过 | 通过 | 421,238 ms | 191,648 |

过程轨迹摘要：

| 任务 | 显式验证闭环 | 验证兜底闭环 | 等价调用拦截 | 收尾模式拦截 | 实施模式拦截 |
| --- | ---: | ---: | ---: | ---: | ---: |
| 账本 | 2 | 0 | 1 | 2 | 0 |
| 配置迁移 | 2 | 0 | 0 | 1 | 3 |
| 依赖规划 | 2 | 0 | 0 | 0 | 1 |

账本结果来自完整三任务回归中的通过运行；配置与依赖结果来自最终修复后的两任务回归。
机器可读结果分别保存在：

- `docs/evaluations/glm-5.2-long-horizon-suite-2026-07-16.json`
- `docs/evaluations/glm-5.2-long-horizon-retry-2026-07-16.json`

前一个文件保留中间完整套件的失败结果，后一个文件记录最终配置与依赖任务 `2/2`
通过。失败数据被保留，没有用最终结果覆盖调试证据。

## 闭环控制器

本轮实现了以下运行时机制：

1. `complete_task` 提供结构化终态，包括摘要、验证证据和剩余工作。
2. `update_plan` 支持 `current_scope_complete` 和验证证据；验证后的最终计划更新可直接结束 Turn。
3. 每个执行切片最多 8 个工具决策轮次，每个 Turn 总计最多 24 轮；切片耗尽后通过
   continuation 显式续接，总预算不会因续接重置。
4. 相同工具与参数在无状态变化时最多执行 3 次，之后返回结构化拒绝结果。
5. 连续 12 次观察工具调用没有工作区修改、且计划仍包含实现任务时进入实施模式；只允许
   `write_file`、`apply_patch`、写入型 `spreadsheet` 或明确结束。成功写入后恢复完整工具集。
6. 终端验证成功后进入收尾模式，只允许 `update_plan` 或 `complete_task`。Provider 即使返回
   未声明工具，执行层也会拒绝。
7. 收尾模式连续违规 2 次时，运行时根据成功验证和持久化计划生成可审计兜底终态；继续/恢复
   请求会把上一阶段延期的步骤视为当前范围。实施模式连续违规 3 次则以“未完成”有界结束，
   不伪造成功。
8. 循环保护状态随审批和预算 continuation 持久化，Server 重启后的新阶段仍由 SQLite
   恢复任务计划和历史轨迹。

## 迭代中发现的问题

本次没有只保留最好结果。迭代过程中观察到：

- 初始版本能通过部分隐藏测试，但会在测试后反复读取和运行命令，最终超时。
- 仅在提示词中缩小工具列表不够，Provider 仍可能返回未声明的 `shell` 调用，因此必须在
  执行层再次校验。
- 过早依据公开测试自动完成会掩盖测试覆盖盲区。一次配置迁移运行公开测试通过，但隐藏检查
  发现 URL 重复端口和尾斜杠错误，最终仅 `7/8`。因此功能 Grader 与闭环状态必须独立。
- 240 秒单 Turn 上限会在代码和测试均完成后留下不足一次 Provider 响应的时间。最终套件采用
  300 秒；这不是放宽成功标准，超时仍然严格失败。
- 按步骤文本判断“Phase 2”为延期项在恢复阶段会误判；最终实现结合当前请求是否为
  `continue/resume/recover` 进行归并。

## 当前判断

可以认为“基础长程任务闭环”已经实现，但还不能称为生产级可靠：

- 当前只有 3 个合成编码任务，每项最终版本只运行 1 次。
- 尚未覆盖审批拒绝、用户取消、Provider 429/5xx、上下文压缩后续接和进程崩溃注入。
- 尚未覆盖浏览器、Excel、文档和多智能体长程任务。
- 未进行同一任务 3 次以上的方差评估，也没有与其他模型或旧策略进行成对比较。
- 公开测试通过不等于功能正确；隐藏 Grader 仍是阻止 false completion 的必要硬门槛。

下一验收目标是扩展到至少 10 个长程任务，每项固定配置运行 3 次，并达到 `>= 90%`
严格任务成功率、`0%` false completion、`0%` 密钥泄漏和 `0%` 超时残留进程。

## 复现命令

运行单个任务：

```powershell
.\scripts\evaluate-long-horizon.ps1 `
  -EnvFile "J:\Project\信贷审核助手\.env" `
  -Profile AUDIT_COPILOT_LLM `
  -ExpectedModel glm-5.2 `
  -TaskManifest scripts\fixtures\long-horizon\config-migration\task.json `
  -TurnTimeoutSeconds 300
```

运行三任务套件：

```powershell
.\scripts\evaluate-long-horizon-suite.ps1 `
  -EnvFile "J:\Project\信贷审核助手\.env" `
  -Profile AUDIT_COPILOT_LLM `
  -ExpectedModel glm-5.2 `
  -Repetitions 1 `
  -TurnTimeoutSeconds 300 `
  -SummaryPath docs\evaluations\glm-5.2-long-horizon-suite.json
```

`.opentopia/evaluations/` 保存本地 SQLite、完整轨迹、日志和隔离工作区，默认不提交。
