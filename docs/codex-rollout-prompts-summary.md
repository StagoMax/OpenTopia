# Codex Rollout 提示词翻译文档 — 内容与格式总结

> 源文件：`codex-rollout-prompts-zh-cn.md`

---

## 一、文档目的

将 Codex（基于 GPT-5 的 Agent）在 **rollout JSONL 日志**中可见的系统提示词（自然语言部分）翻译为中文，保留原始标签、标识符、工具名、配置键和路径。完全重复的注入只保留一份译文。

### 数据来源

- `rollout-2026-07-20T19-06-26-019f7f34-9396-76b2-85f3-2d4cc1e34ea0.jsonl`
- `rollout-2026-07-19T18-07-02-019f79d7-ee10-7f21-9eb4-4122c2c15dbe.jsonl`

---

## 二、核心内容

### 1. `base_instructions` — 基础人格与行为准则

| 子主题 | 要点 |
|--------|------|
| **个性** | 好奇心强、有主见的沟通者，像老朋友一样自然对话 |
| **写作风格** | 少用粗体/标题/列表，使用 CommonMark 标准，简洁优先 |
| **技术沟通** | 先说结论再解释步骤，按用户背景调整复杂度，偏好朴素语言 |
| **与用户协作** | 两个沟通频道：`commentary`（进度更新）、`final`（最终回复）；支持上下文压缩后自动恢复 |
| **中间进度** | 每 60 秒内向 commentary 发一次更新；阻塞性/澄清问题放 final |
| **最终答案** | 聚焦最重要信息，使用最少格式；引用文件用可点击 Markdown 链接 |
| **可视化** | 仅当能显著简化理解时使用，优先用最小可视化形式 |
| **文件编辑** | 使用 `apply_patch`，禁用 `git reset --hard` 等破坏性命令 |
| **自主性** | 按请求类型（回答/诊断/修改/监控）采取不同行为，不越权 |
| **Skills** | 读取 `SKILL.md` 后使用，用户点名必须用，渐进披露避免加载无关资源 |

### 2. 第一组 `developer` 消息 — 环境与权限

| 子部分 | 要点 |
|--------|------|
| **权限指令** | 沙箱模式 `danger-full-access`（无限制）；审批策略 `never` |
| **桌面端上下文** | 支持图片/视频/Mermaid 图表、工作区依赖、自动化（周期性任务）、线程管理、行内代码评论、Git 操作指令 |
| **协作模式** | **Default 模式**：优先合理假设并执行，减少提问；避免多 Agent 委派 |
| **插件** | 插件提供 Skills / MCP / 应用；用户点名时优先使用 |
| **可用 Skills** | 列出 30+ Skill，覆盖：图片生成、文档处理、电子表格、PPT、PDF、GitHub、Linear、飞书、安全扫描、UI/UX、可视化、Chrome 控制等 |

### 3. 第二组 `developer` 消息 — 多 Agent 团队

> 当前 Agent 身份为 `/root`（主 Agent）

- 可生成子 Agent，最多 **4 个并发槽位**
- 协作工具：`spawn_agent`、`followup_task`、`send_message`、`wait_agent`、`interrupt_agent`、`list_agents`
- 所有 Agent **共享同一目录和文件系统**
- 完整历史派生继承父 Agent 的模型和推理强度

### 4. 第三组 `developer` 消息 — 多 Agent 模式覆盖

- 除非用户或 `AGENTS.md` / Skill 指令明确要求，否则**不生成子 Agent**

### 5. 环境注入

| 类型 | 内容 |
|------|------|
| **首轮** | cwd、shell、日期、时区、工作区根目录、权限配置 |
| **后续动态** | 日期更新、visualizations 路径更新、新增工作区根目录 |
| **增量更新** | 不重复列出不变的字段 |

### 6. 动态 Skill 注入差异

- 两个版本（15,068 字符 vs 15,117 字符）的 `openai-docs` 描述略有不同
- 各日志行中 `chrome`、`computer-use` 等插件的版本号有差异（以表格对比）

### 7. `turn_context` 字段含义

| 字段 | 含义 |
|------|------|
| `turn_id` | 本轮唯一 ID |
| `cwd` | 当前工作目录 |
| `workspace_roots` | 工作区根目录列表 |
| `current_date` / `timezone` | 当前日期 / 时区 |
| `approval_policy` | 审批策略（`never`） |
| `sandbox_policy.type` | 沙箱策略（`danger-full-access`） |
| `model` | 模型（`gpt-5.6-sol`） |
| `personality` | 个性（`friendly`） |
| `collaboration_mode.mode` | 协作模式（`default`） |
| `reasoning_effort` | 推理强度（`xhigh`） |
| `multi_agent_mode` | 多 Agent 启动模式（`explicitRequestOnly`） |

### 8. 两份日志的实际记录顺序

- **7/20 日志**：5 轮（含 compacted 恢复），每轮以 `turn_context` 为界
- **7/19 日志**：多轮 + 多次 compacted 恢复，其中 `type: "compaction"` 的摘要仅以 `encrypted_content` 存在（无明文）

---

## 三、格式特点与问题

| 特点 | 说明 |
|------|------|
| **标题层级** | 使用 H1-H3，但 H2 与 H3 之间有时缺少一致性（如 "Git" 条目与其他子条目层级不统一） |
| **引用方式** | 使用 `code` 标记标识符、工具名、路径；XML 环境片段用代码块包裹 |
| **表格使用** | 部分结构化信息使用表格（版本差异、日志行映射），格式较清晰 |
| **列表** | 使用无序列表描述要点，但嵌套深度和一致性有时不一致 |
| **内容重复** | 部分 Skills 列表重复出现在不同位置；英文原版与中文译文的排版关系可以进一步优化 |
| **行文长度** | 单条翻译/说明普遍较长，部分段落可进一步分段提升可读性 |

---

## 四、改进建议

1. **统一子标题层级** — 将分散的 `##` 条目按逻辑归入更大的 H1/H2 结构下
2. **减少内联引用冗余** — 对英文路径/标识符，可在首次出现后统一缩写或索引
3. **表格与列表的一致性** — 同类信息统一使用表格或列表，不混用
4. **Skill 列表单独成页** — 30+ 个 Skill 的描述可移出主文档作为附录，正文只保留索引
5. **增加导航目录** — 文档较长，可添加 TOC（Table of Contents）
