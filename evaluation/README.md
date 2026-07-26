# Application Agent Evaluation Harness

这是一个与 OpenTopia 产品运行时解耦的应用级 Agent 评测 Harness。它不 import `apps/`、`crates/` 或任何产品内部模块，目标应用只通过一个外部进程 Adapter 接入。

```text
Task/Suite JSON -> Harness Runner -> Target Adapter -> 被测应用
                         |                 |
                         |                 +-- stdin / file prompt
                         |                 +-- AGENT_EVAL_* environment
                         |                 +-- JSONL event file
                         |
                         +-- isolated workspace
                         +-- external graders
                         +-- result.json / summary.json / report.md
```

## 快速开始

在仓库根目录执行：

```powershell
node --test evaluation/test/validation.test.mjs evaluation/test/runner.test.mjs
node evaluation/src/cli.mjs validate `
  --suite evaluation/examples/smoke/suite.json `
  --target evaluation/examples/smoke/target.json
node evaluation/src/cli.mjs run `
  --suite evaluation/examples/smoke/suite.json `
  --target evaluation/examples/smoke/target.json `
  --output evaluation/.runs
```

也可以在 `evaluation/` 目录中执行 `npm test`。Harness 只使用 Node 22+ 内置模块，不需要把依赖加入根 `package.json` 或 pnpm workspace。

## 接入任意应用

### Target 配置

Target 是独立于 Task 的黑盒适配器：

```json
{
  "schemaVersion": 1,
  "id": "my-agent-http-adapter",
  "command": "node",
  "args": ["adapter.mjs"],
  "cwd": "{targetDir}",
  "env": {
    "MY_AGENT_BASE_URL": "http://127.0.0.1:9000"
  },
  "passEnvironment": ["MY_AGENT_API_TOKEN"],
  "inheritEnvironment": false,
  "promptTransport": "stdin",
  "eventTransport": "jsonl-file"
}
```

默认不继承评测进程环境。`passEnvironment` 是显式白名单，适合传递 API Token；不要使用 `inheritEnvironment: true` 运行正式安全评测。

Runner 会提供以下变量：

| 变量 | 含义 |
|---|---|
| `AGENT_EVAL_RUN_ID` | 批次 ID |
| `AGENT_EVAL_TRIAL_ID` | 试验 ID |
| `AGENT_EVAL_TASK_ID` | 任务 ID |
| `AGENT_EVAL_WORKSPACE` | 被测应用可以操作的隔离工作区 |
| `AGENT_EVAL_PROMPT_FILE` | 任务 Prompt 文件 |
| `AGENT_EVAL_EVENTS_PATH` | Adapter 必须写入的 JSONL 事件路径 |
| `AGENT_EVAL_TRIAL_DIR` | 本次 Trial 的外部目录 |
| `AGENT_EVAL_SECRET_CANARIES` | 仅在任务显式开启时注入的安全 Canary |

Adapter 使用参数数组启动，不经过 Shell。退出码为 0 表示 Adapter 认为应用任务结束；Harness 仍以 Grader 和安全检查为最终依据。

### JSONL 事件

Adapter 每行写一个事件。Runner 会补齐缺省的 Run/Trial/Task 身份字段并校验：

```json
{
  "schemaVersion": 1,
  "timestamp": "2026-07-25T05:00:00.000Z",
  "source": "my-agent-adapter",
  "type": "model.usage",
  "payload": {
    "inputTokens": 1200,
    "outputTokens": 240,
    "totalTokens": 1440,
    "cachedInputTokens": 800,
    "cacheWriteTokens": 100,
    "cacheSupport": "provider_reported"
  }
}
```

常用事件类型：

- `tool.call.started` / `tool.call.completed`，payload 至少包含 `name`；
- `skill.selected`，payload 包含 `name`；
- `mcp.call.completed`，payload 包含 `server`；
- `plugin.capability.used`，payload 包含 `plugin`；
- `model.usage`，使用供应商上报的输入、输出、总量和 Cache 字段；
- `phase.completed`、`context.compaction.completed`、`agent.completion.claimed`；
- `browser.action.completed`，payload 可包含 `valid`、`targetCorrect`、`recovered`；
- `subagent.spawned`、`subagent.completed`、`subagent.cancelled`；
- `memory.assertion`，payload 包含 `passed` 和能力分类；
- `security.violation`，任何命中均按硬门槛处理。

### Task 与 Grader

Task 只描述用户任务、Fixture、能力策略、预算和评分约束。隐藏命令 Grader 位于 Task 文件目录中，但永远不复制到 Agent 的 `workspace`。命令 Grader 约定：

- 退出码 0：检查通过；
- 退出码 1：任务结果不通过；
- 退出码 2 或启动/超时错误：Grader 基础设施错误。

文件 Grader 支持存在性、类型、全文、包含/排除文本、SHA-256 和 JSON 路径断言。能力策略支持 `mustUse`、`mustNotUse`、`optional` 和 `oneOf`。

## OpenTopia 适配器

`adapters/opentopia-http.mjs` 是一个可选的产品适配器，不属于 Harness 核心。它只调用 OpenTopia Server 的 HTTP API，读取线程事件并转换成 Harness JSONL，不引用 Rust 或 Desktop 源码。

先启动一个用于评测的独立 OpenTopia Server，再复制并调整 `examples/opentopia-target.example.json`：

```powershell
$env:OPENTOPIA_API_TOKEN = "<evaluation-token>"
node evaluation/src/cli.mjs run `
  --suite <suite.json> `
  --target evaluation/examples/opentopia-target.example.json `
  --output evaluation/.runs
```

该 Adapter 默认拒绝审批（`OPENTOPIA_EVAL_APPROVAL_MODE=deny`）。安全任务应保持默认值；只有明确测试“批准后行为”的 Task 才使用独立的显式配置打开批准。

现有 `scripts/evaluate-long-horizon*.ps1` 保持不变。迁移旧 Fixture 时，应把它们转换成新 Task/Grader 格式，而不是让新 Harness import 或修改旧脚本。

## 结果和安全属性

每次 Run 生成：

- `manifest.json`：Suite、Target、Task 哈希、平台、Node 版本和重复次数；
- `trials/<id>/result.json`：结果、分项分数、检查、Token/Cache 和领域指标；
- `trials/<id>/events.jsonl`：脱敏后的标准事件；
- `summary.json`：批次聚合；
- `report.md`：人工审阅报告。

状态区分 `passed`、`task_failed`、`false_completion`、`safety_violation`、`watchdog_timeout`、`application_crash`、`grader_error` 和 `infra_error`。基础设施错误不进入能力成功率分母，但会被单独报告。

KV Cache 只有在事件包含 `cacheSupport: "provider_reported"` 或供应商 Cache 字段时才统计为命中。没有供应商字段时，结果是“不支持”，不是 0%。

## 分阶段扩展

当前版本已实现：

- 独立 Task/Target/Event/Result 契约；
- 隔离工作区、隐藏 Grader、进程看门狗和最小环境；
- 最终文件/命令、轨迹、能力路由、安全、Token/Cache 评分；
- 浏览器、多 Agent、记忆事件指标；
- OpenTopia HTTP 黑盒适配器和确定性 Mock Target。

后续按评测文档扩展：

1. 把现有三类长程 Fixture 转成新 schema，并增加重启/压缩/故障注入 Adapter；
2. 增加 BrowserGym/WebArena 风格的本地可重置站点与后端状态 Grader；
3. 增加 Desktop/Playwright Adapter 和截图/Canvas 检查；
4. 增加跨运行配置矩阵、冷/热 Cache 配对和基线回归；
5. 把私有 Suite、发布 holdout 和安全 Canary 放到 Harness 外部的受控数据仓库。
