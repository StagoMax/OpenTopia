# OpenTopia Prompt Runtime 改造说明

## 1. 改造结果

本次改造把 OpenTopia 的 Agent 指令从“基础提示词加若干服务端追加文本”升级为可配置、可观测、可验证的 Prompt Runtime。

核心目标不是逐字复制 Codex，而是复用它已经验证过的上下文工程原则，同时保留 OpenTopia 更适合本机工作台的执行架构：

- 固定规则、条件模块和每轮动态状态分层装配。
- 用户设置直接映射到运行时策略模块。
- 文字策略、工具能力、权限审批、OS 沙箱和网络策略彼此独立。
- 多 Agent 设置同时控制提示词和真实工具目录，避免伪开关。
- 每个模块带装配元数据，进入现有上下文快照和活动时间线。
- Prompt cache key 包含运行时设置哈希，设置变化不会错误复用旧前缀。

相关的 Codex 原始拆解见 [codex-system-prompt-modular-analysis-zh-cn.md](./codex-system-prompt-modular-analysis-zh-cn.md)。

## 2. 新的装配模型

每轮有效上下文可以表示为：

```text
EffectiveContext(turn) =
    FixedCore
  + ConditionalRuntimeModules(agentRuntime, surface, capabilities)
  + ExperienceMode(thread)
  + RepositoryInstructions(AGENTS.md)
  + PermissionPolicy(permission, sandbox, network)
  + PluginAndSkillCatalog(installed, enabled, selected)
  + WorldState(workspace, git, tools, date, platform)
  + ConversationOrSummary
  + UserMessage

AvailableActions(turn) =
    BuiltinTools
  + EnabledMcpTools
  - ModeDeniedTools
  - RuntimePolicyDeniedTools

ExecutableBehavior =
    PromptAllowed
  ∩ AvailableActions
  ∩ UserAuthorization
  ∩ PolicyDecision
  ∩ OsSandbox
  ∩ NetworkPolicy
```

最后一个交集关系是本次设计的安全核心。任何单一开关都不能独自扩大完整执行范围。

## 3. 固定、条件与动态模块

| 装配类别 | OpenTopia 模块 | 变化来源 | 普通用户是否直接编辑 |
|---|---|---|---|
| 固定 | `base_contract` | 产品版本 | 否 |
| 固定 | `skills_protocol` | 产品版本 | 否 |
| 条件 | `personality` | `agentRuntime.personality` | 是 |
| 条件 | `autonomy` | `agentRuntime.autonomy` | 是 |
| 条件 | `progress_updates` | `agentRuntime.progressUpdates` | 是 |
| 条件 | `multi_agent_policy` | 设置与调度器能力 | 是 |
| 条件 | `desktop_protocol` | 当前运行界面 | 间接 |
| 条件 | `experience_mode` | 任务的 Work / Code 模式 | 是 |
| 动态 | `workspace_scope` | 工作区与可读写根目录 | 间接 |
| 动态 | `permission_policy` | 审批、沙箱、网络 | 是 |
| 动态 | repository instructions | 当前目录的 `AGENTS.md` 链 | 通过文件 |
| 动态 | plugin / Skill modules | 安装、启用和逐轮选择 | 是 |
| 动态 | world state | Git、工具、日期、平台和运行时能力 | 否 |

每个 Prompt Runtime 模块带有 `promptModuleId`、`assemblyClass`、`selectedBy`、`settingValue` 和 `editable` 元数据。多 Agent 模块还记录真实能力是否存在、并发上限和用户输入工具是否可用。

## 4. 用户可配置策略

### 4.1 沟通风格

- `focused`：压缩叙述，突出结果和关键证据。
- `professional`：默认值，解释重要判断与取舍，省略例行细节。
- `warm`：增加引导性和自然感，但不牺牲准确性和独立判断。

### 4.2 自治程度

- `guided`：重要设计选择、广泛重构、外部写入等动作前优先确认。
- `balanced`：默认值，在已授权范围内完成常规实现，只确认会显著改变结果的选择。
- `proactive`：自主解决可逆歧义并推进到验证，只在真实授权边界或高影响决策处停止。

自治程度只改变“何时询问”，不改变“是否有权执行”。

### 4.3 多 Agent

- `off`：从模型工具目录移除全部子 Agent 工具，并注入明确禁用策略。
- `explicit`：默认值，仅在用户、仓库规则或已加载 Skill 明确要求时委派。
- `adaptive`：当子任务边界清晰、可独立并行且收益大于协调成本时允许主动委派。

这比单纯用提示词限制更可靠，因为 `off` 模式在 capability 层面不可调用委派工具。

### 4.4 进度更新

- `milestones`：只在阶段完成、重要变化或阻塞时同步。
- `balanced`：默认值，报告重要发现、决策、阶段完成和验证结果。
- `frequent`：在每个有意义的工作转换点同步。

## 5. 模仿 Codex 的部分

| Codex 原理 | OpenTopia 落地 |
|---|---|
| 系统提示词不是单一长文本，而是运行时装配 | 新增 `prompt_runtime.rs`，集中编译固定与条件模块 |
| 文字提示词和工具 schema 是并列输入 | 多 Agent 设置同时影响策略文本和 `provider_tool_catalog` |
| Skills 目录只是路由信息，使用前需加载完整规则 | 固定 `skills_protocol` 明确目录、加载和回退协议 |
| 不同宿主按需注入不同规则 | Desktop 才注入 `desktop_protocol` |
| 权限、沙箱和网络需要逐轮声明 | 使用结构化 `permission_policy` 模块注入 |
| 长任务持续同步进度 | 将更新密度改为用户可配置策略 |
| 上下文压缩后恢复关键约束 | 模块进入现有 `CompiledModelContext` 与快照链路 |

## 6. 没有照搬 Codex 的部分

### 6.1 不使用 `::directive` 驱动 UI

Codex Desktop 能解析特殊文本指令来改变应用状态。OpenTopia 已有 typed event、artifact、preview、approval 和 tool result 数据模型。继续使用结构化事实来源更合适：

- 不依赖 Markdown 文本解析副作用。
- 状态可持久化、重放、审计和测试。
- 不同模型供应商无需学习 Codex 私有指令格式。
- UI 不会因为模型“声称完成”就误认为应用状态已经变化。

因此 Desktop 协议明确禁止模型伪造 Codex directive，并要求只报告 OpenTopia 已观测到的状态。

### 6.2 不用提示词代替 OS 沙箱

OpenTopia 已有真实的本机沙箱装配、Guardian 和策略引擎。提示词只负责告诉模型当前边界，不能成为安全执行器。采用顺序是：

```text
用户授权 -> Policy Engine / Guardian -> Tool Execution -> OS Sandbox -> Result Event
```

即使设置为主动自治或自适应多 Agent，以上链路也不会被绕过。

### 6.3 不把 Codex 的 task/thread 产品语义搬入 OpenTopia

OpenTopia 已有自己的任务、会话、计划、目标、子 Agent 和恢复模型。本次只吸收“分层上下文”和“显式协议”，不替换本机 SQLite 数据模型，避免为了外观相似破坏恢复能力和历史兼容性。

## 7. 超越 Codex 样本的部分

### 7.1 装配元数据可观测

Codex rollout 展示了最终拼装结果，但普通应用很难解释某段规则从何而来。OpenTopia 为模块附加装配类别、选择来源、当前值和可编辑状态，现有活动时间线可以查看每轮上下文项。

### 7.2 策略与能力双重执行

多 Agent `off` 不只是自然语言约束，而是实际移除工具。该模式避免模型忽略软提示后仍发出调用。

### 7.3 缓存与配置一致

自动 Prompt cache key 加入 `AgentRuntimeSettings` 内容哈希。沟通风格、自治或委派策略变化后，会生成新的缓存身份，不会将旧策略前缀误认为兼容。

### 7.4 自适应委派

Codex 样本的策略是只有显式请求才允许委派。OpenTopia 保留该模式作为默认，同时提供 `adaptive`：只有独立性和并行收益足够明确时才可主动委派，并仍受共享文件系统、并发限制和完成等待规则约束。

## 8. 兼容性与默认值

旧设置文件没有 `agentRuntime` 时由 Serde 默认值自动迁移，不要求手工修改：

```json
{
  "personality": "professional",
  "autonomy": "balanced",
  "multiAgent": "explicit",
  "progressUpdates": "balanced"
}
```

默认选择偏保守：保留专业、平衡的执行体验，多 Agent 必须有明确触发来源。用户主动选择 `adaptive` 后才启用收益驱动的内部并行。

## 9. 主要实现位置

- `crates/opentopia-core/src/prompt_runtime.rs`：设置类型、模块编译器和装配元数据。
- `crates/opentopia-core/src/agent.rs`：运行时设置、能力探测、工具过滤和默认上下文装配。
- `crates/opentopia-core/src/settings.rs`：持久化设置与旧配置兼容。
- `crates/opentopia-server/src/main.rs`：每轮上下文、世界状态、缓存键和设置 API。
- `apps/desktop/src/components/SettingsPanel.tsx`：智能体策略设置页。
- `apps/desktop/src/styles/app.css`：桌面和窄屏布局。

## 10. 验证标准

实现需要同时满足以下条件：

1. 旧设置能加载并获得默认 Agent Runtime。
2. 四组设置能经桌面 API 保存并回读。
3. 每轮上下文包含正确模块和装配元数据。
4. 多 Agent `off` 时工具目录没有 `spawn_agent` 等工具。
5. 设置变化会改变上下文哈希和自动缓存键。
6. 权限文本明确区分审批、沙箱和网络。
7. Rust 编译与单元测试、TypeScript 类型检查和桌面构建通过。
8. 设置页在桌面与窄屏下无溢出、遮挡和不可操作控件。
