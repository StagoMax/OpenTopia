# GLM-5.2 长程任务能力评测（2026-07-16）

## 评测范围

本次评测是 OpenTopia 本地运行的确定性长程任务评测。它不是 SWE-bench
或 Terminal-Bench 的官方成绩，不应与这些基准的排行榜分数直接比较。
基于 Docker 的官方评测框架仍不在当前本地 MVP 的范围内。

评测设计参考了两种权威基准的方法：

- [SWE-bench](https://github.com/SWE-bench/SWE-bench)：智能体需要在真实代码仓库中
  解决实际 Issue，并由确定性测试判定结果。其方法详见
  [ICLR 2024 论文](https://openreview.net/forum?id=VTF8yNQM66)。
- [Terminal-Bench 3](https://github.com/harbor-framework/terminal-bench-3)：在隔离环境中执行
  贴近实际工作的终端任务，使用经过参考答案验证的任务和客观测试脚本进行评分。
  其基准论文可在 [OpenReview](https://openreview.net/forum?id=a7Qa4CcHak) 查看。

## 评测流程

评测入口为 `scripts/evaluate-long-horizon.ps1`。脚本会基于
`scripts/fixtures/long-horizon/seed` 创建全新的 Git 测试仓库，先确认初始代码无法
通过测试，再使用智能体无权修改的外部隐藏评分器进行判定。

1. 探测已配置的 OpenAI 兼容服务：模型列表、SSE 文本流、自动工具调用、单次调用
   continuation、严格序列化历史诊断，以及压缩历史兼容性。
2. 阶段 1 要求智能体实现 CSV 账本库并运行公开测试。
3. 如果阶段 1 成功，则使用同一个 SQLite 数据库重启 OpenTopia Server。
4. 阶段 2 要求恢复持久化的任务与计划，并实现命令行工具。
5. 检查受保护文件哈希、6 项库功能、2 项 CLI 功能、计划与测试过程要求、重启恢复，
   并执行逐字节密钥扫描。
6. 每个阶段最多运行 420 秒。即使部分测试通过，超时仍判定为失败。

API Key 仅由执行探测和评测的进程从 `J:\Project\信贷审核助手\.env` 读取。
提交到仓库的报告不包含密钥。桌面端使用 Electron `safeStorage` 保存密钥，
渲染进程只能读取配置元数据。

## 评测结果

机器可读结果：
`docs/evaluations/glm-5.2-long-horizon-2026-07-16.json`。

| 指标 | 结果 |
| --- | --- |
| 运行 ID | `glm-5-2-20260716T075801Z` |
| 总体结果 | **失败** |
| 服务模型列表/文本/工具调用/continuation | 通过 |
| 严格序列化的多工具历史 | 外部网关返回 HTTP 400 |
| 压缩工具历史兼容性 | HTTP 200 |
| 初始库测试/完整测试 | 2/6 和 3/8，符合预期的失败基线 |
| 最终库评分 | 6/6 通过 |
| 最终完整评分 | 7/8 通过 |
| 未通过项 | CLI 成功契约 |
| 工具调用 | 启动 42 次，完成 42 次 |
| 执行切片 | 4 个（`turn_started` 事件） |
| 预算检查点 | 3 个 |
| 计划更新 | 5 次 |
| 测试命令调用 | 6 次 |
| Token 用量 | 输入 442,160；输出 25,267；总计 467,427 |
| 耗时 | 441,958 毫秒 |
| 密钥扫描 | 通过 |
| 最终失败原因 | 阶段 1 超过 420 秒硬超时限制 |

智能体完成了可用的账本库，并通过全部隐藏库功能检查，但在阶段目标已经满足后，
仍反复读取文件、更新计划和运行测试，没有及时结束阶段 1。因此评测未能进入重启恢复
和 CLI 阶段。结果说明系统已经具备有效的编码能力，但长程任务的可靠闭环能力仍不达标。

## 工程结论

1. 仅通过一次聊天请求不足以验证服务协议兼容性。当前探测已覆盖工具结果
   continuation 和压缩历史。
2. 智能体需要显式的阶段完成控制器：当必需测试通过且当前范围内的计划步骤完成后，
   应立即结束当前执行切片。
3. 需要在完整轨迹层面去重重复读取和测试命令，或为重复工具调用设置预算。本次小型任务
   累计消耗了 467k Token。
4. 压缩后的服务历史应采用有界、增量传输，避免反复发送此前的全部工具输出。
5. 由于阶段 1 未正常结束，本次没有测量重启恢复能力；该项仍需保留为待验收事项。

## 复现方式

```powershell
powershell -ExecutionPolicy Bypass -File scripts/evaluate-long-horizon.ps1 `
  -EnvFile "J:\Project\信贷审核助手\.env" `
  -Profile AUDIT_COPILOT_LLM `
  -ExpectedModel glm-5.2 `
  -SummaryPath "docs\evaluations\glm-5.2-long-horizon-2026-07-16.json"
```

`.opentopia/evaluations/` 下的评测输出会被有意忽略，不提交到仓库。该目录包含用于
调试的本地数据库、执行轨迹、日志和工作区。
