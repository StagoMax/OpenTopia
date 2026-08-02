# OpenTopia Harness 内置能力与插件边界设计

状态：实现基线，核心底座已落地，贡献运行时仍在补齐
日期：2026-08-01
目标读者：OpenTopia 维护者、插件作者、后续并行实现 Agent

本文使用以下状态标记：

- **已完成**：已有持久化、API 或运行时接线，并有对应测试。
- **协议底座**：类型、注册、选择或宿主边界已经存在，但尚未形成完整用户工作流。
- **未完成**：仍缺少关键运行时、桌面入口、厂商适配或端到端验收。
- **兼容层**：新协议已接入，但仍保留旧状态或旧调用路径，尚不能删除。

## 1. 目标

本文定义 OpenTopia harness 中哪些能力必须由应用内置，哪些能力应作为插件提供，
以及平台维护者、插件作者和最终用户分别能够配置什么。

设计首先解决一个容易混淆的问题：GitHub 插件不是 Git 实现。OpenTopia 需要内置一套
与远程厂商无关的本地 Git 能力；GitHub、GitLab、Gitee、Bitbucket、Azure DevOps、
Gerrit 等提供的是代码托管服务连接器。连接器可以扩展 pull request、issue、CI、review
和 release 等远程工作流，但不应替代本地仓库、index、worktree、diff 和 commit 的宿主实现。

本文的实现目标包括：

- 固定 harness 内核和安全边界，避免为了插件化而暴露内部对象。
- 保持现有 Codex-compatible `skills`、`mcpServers` 和 `apps` manifest 能力。
- 支持官方捆绑插件、第三方普通插件和高信任驱动三种不同等级。
- 让用户控制安装、生效范围、参数和可选授权，但不能突破平台权限上限。
- 给后续多 Agent 实现提供可拆分的协议、数据模型、迁移顺序和验收条件。

## 2. 术语与分层

### 2.1 Harness Kernel

Harness Kernel 是所有任务都依赖的稳定协议和安全边界，包括：

- Agent turn loop、模型工具调用编排、取消和 continuation。
- Thread、Message、Turn、Event、Approval、Goal 和 Task Plan 状态。
- 上下文编译、历史投影、compaction、provider state 和 prompt cache 协议。
- Policy、approval、sandbox、审计和资源限制的最终裁决。
- Tool、MCP、Skill、Plugin 的发现和装配机制。
- 子 Agent 调度、身份、mailbox、等待和生命周期。

Kernel 不作为普通插件提供，也不允许插件替换其最终安全判断。

### 2.2 Host Service

Host Service 是应用内置、可被桌面 UI、Agent 工具和插件共同调用的本地服务。它提供
稳定的能力接口，但具体 UI 或领域工作流可以由插件贡献。首批 Host Service 包括：

- Workspace/File Service。
- Execution/Patch Service。
- Local Git Service。
- Artifact/Preview Host。
- Secret Broker。
- Policy/Approval Broker。
- Event/Audit Service。
- Plugin/Capability Registry。

Host Service 不是可卸载插件。插件只能通过收敛后的协议使用它，不能获得 Rust 内部
`ToolContext`、`SessionStore` 或 `AgentCore` 的直接引用。

### 2.3 Bundled Plugin

Bundled Plugin 是由 OpenTopia 随安装包分发和测试的可选能力，例如 Spreadsheet、Browser
Automation 和 Computer Use。它在产品层面可以表现为“官方功能”，但在架构层面仍走插件
注册、启停、权限和健康检查协议，以验证插件机制并降低 core 耦合。

Bundled 不等于自动授权。高风险能力仍需要用户启用并经过运行时审批。

### 2.4 Standard Plugin

Standard Plugin 是用户安装的普通插件，主要由 Skills、MCP servers、受限 App views、
Agent profiles、预览处理器和配置 schema 组成。默认采用进程隔离，不加载 Rust 动态库，
不能直接读取 Provider 密钥，也不能修改安全策略。

### 2.5 Trusted Driver

Trusted Driver 可以接触模型传输、桌面控制、执行环境或安全评审等高信任面，例如 Provider
driver、Computer backend 和将来的 remote execution backend。它必须由 OpenTopia 内置、
签名或经开发者模式显式信任。用户不能通过普通插件安装流程把插件提升为 Trusted Driver。

### 2.6 Code Host Connector

Code Host Connector 是面向 GitHub、GitLab 等远程服务的插件类别。本文统一使用
“代码托管连接器”或 `scm_connector`，避免使用含混的“Git 插件”。

## 3. 配置责任

这里必须区分“谁定义平台能力”和“谁在使用产品时选择参数”。在产品没有独立部署管理员时，
应用开发者也可能暂时代行管理员职责，但两类配置不能因此混为一层。

### 3.1 应用开发者/平台维护者配置（由 OpenTopia 代码和发布包决定）

这是“应用内置配置”，随代码、签名元数据或发布渠道交付，最终用户和普通插件不能修改：

- 插件 API、manifest schema 版本和 contribution 输入输出协议。
- 插件信任等级、官方来源校验和允许的运行方式；信任信息由宿主记录，不能由 manifest 自报。
- 文件系统、网络、桌面、进程、secret、沙箱、审批和审计的权限上限。
- 超时、消息大小、输出大小、资源配额、路径保护和 fail-closed 行为。
- App View 的 CSP、sandbox、允许的消息 channel，以及禁止的 Node/Electron 能力。
- 哪些插件随安装包分发、默认安装和默认启用。
- Provider、Computer、Execution、Reviewer 等 Trusted Driver 的宿主注册表。
- Kernel prompt、安全 prompt、Local Git 语义和不可覆盖的行为约束。

当前实现中，Spreadsheet、Browser Automation、Computer Use 的官方信任级别、版本、
`default_enabled` 和 native capability 映射都保存在宿主拥有的 Bundled Plugin catalog 中，
而不是插件自己的 `plugin.json` 中。

三个 Bundled 包使用与安装绝对路径无关的宿主身份 `bundled:spreadsheet`、
`bundled:browser-automation`、`bundled:computer-use`；其 contribution ID 稳定为
`<plugin-id>/<local-id>`，因此应用升级或移动安装目录不会让 activation、permission 和 settings
记录失联。

三者都随应用默认安装，但安装不等于启用或授权：Spreadsheet 默认启用，只有在其 manifest
申请的 workspace 文件权限已授予后才进入 active snapshot；Browser Automation 和 Computer Use
默认关闭，必须由用户显式启用并授予所需权限。三者的实际调用仍继续经过 Policy、Approval、
Sandbox 和审计，插件 activation/permission grant 不能扩大这些平台上限。

### 3.2 插件作者声明（由插件包决定）

插件作者在 manifest 和配置 schema 中声明：

- 插件元数据、版本和最低宿主 API 版本。
- Skills、MCP servers、Apps、Agent profiles 等 contributions。
- 所需权限和所需 Host Service 能力。
- 用户可配置字段、默认值、校验规则和 secret 字段标记。
- activation 条件、健康检查和卸载清理声明。
- 代码托管连接器支持的远程 URL 类型和功能矩阵。

插件只能“申请”权限，不能给自己授权；manifest 中也不允许声明自身信任等级。

### 3.3 部署管理员/Workspace Owner 配置（组织或项目治理）

这一层负责在平台上限内设置团队或项目约束，典型内容包括：

- 允许的插件来源、版本锁定、自动更新策略和组织级禁用列表。
- 全局或 Workspace 范围 activation；窄作用域只能继续收紧，不能绕过上层禁用。
- Workspace 级普通 settings、opaque secret binding、网络域名和文件访问约束。
- 可授予权限的上限、预批准的低风险权限，以及必须逐次审批的高风险能力。
- Workspace 的代码托管账户、remote 到 connector/account 的默认绑定。
- Browser profile、下载策略和 Computer 可选窗口类型的组织上限。

**当前状态：协议底座。** Server 和 Desktop 已有 global/workspace/thread 三层 activation、settings、
permission grant/revoke 和 opaque secret binding，但尚无独立管理员身份、组织策略、RBAC 或“谁有权
写 global/workspace 设置”的后端鉴权。当前作用域模型不能被误写成已经完成管理员治理。

### 3.4 最终用户配置（个人和当前任务选择）

最终用户可以在管理员和平台允许范围内配置：

- 安装或卸载个人插件，以及启用、禁用当前可管理的插件。
- 个人全局设置、当前 Workspace 设置和 Thread activation。
- 选择本轮使用的 Skills 和 Agent profile。
- 插件 schema 明确开放的普通参数；secret 只能选择 opaque binding ID。
- Provider 连接、模型、Endpoint 和凭据绑定。
- Browser profile、下载目录、域名授权和 Computer 窗口范围。
- MCP server 启停和显式环境变量绑定。
- 为某个 Git remote 选择代码托管 connector 和账户绑定。
- 查看权限用途，授予或撤销产品允许由自己决定的权限。

最终用户不能配置：

- 绕过 Policy、Approval、Sandbox 或审计。
- 让普通插件读取任意目录、其他插件配置或 Provider 密钥。
- 把普通插件升级为 Trusted Driver。
- 修改或删除历史审批、工具事件和审计记录。
- 用插件 prompt 覆盖 Kernel 安全指令。
- 允许 App view 在 renderer 中执行不受限 Node/Electron API。

### 3.5 配置优先级

运行时采用以下优先级，前者是约束，后者只能在约束内细化：

1. 平台不可变安全约束。
2. 安装来源对应的信任策略。
3. 插件 manifest 的能力和默认值。
4. 部署管理员/组织策略。
5. 用户全局设置。
6. Workspace Owner 管理的 Workspace 设置。
7. Thread activation 和本轮选择。
8. 每次高风险调用的即时审批结果。

更具体的配置不能扩大上一层授予的权限。例如 Thread 可以关闭 Browser，但不能在全局
禁止桌面控制时重新开启 Computer Use。

## 4. Git 的明确边界

### 4.1 应用内置：Local Git Service

以下能力属于 OpenTopia 应用内置功能，不抽成 GitHub 或其他厂商插件：

| 能力 | 所属 | 说明 |
| --- | --- | --- |
| 仓库发现和 root 解析 | Host Service | 为 workspace、world state 和变更追踪提供稳定事实 |
| branch、HEAD、status、remote 列表 | Host Service | provider-neutral，只读取本地 Git |
| staged/unstaged/untracked/conflict 状态 | Host Service | Diff Review 和 turn change tracking 的基础 |
| diff、hunk 解析、stage、unstage、discard | Host Service | 必须统一经过路径校验、审批和审计 |
| branch list/create/switch | Host Service | 本地 refs 操作，不属于 GitHub |
| commit | Host Service | 创建本地 Git commit，与远程厂商无关 |
| worktree create/list/remove | Host Service | Agent 隔离工作区和并行任务基础能力 |
| fetch、pull、push | Host Service | 使用用户已有的 Git remote/auth，协议上不假设厂商 |
| Git world state | Harness Kernel | branch/status 摘要进入模型上下文 |
| `.git` 保护 | Policy/Sandbox | 插件和工具不能绕过受保护元数据策略 |
| turn changes 和 undo | Harness Kernel | 不能依赖某个远程连接器是否安装 |

桌面中的 Git 状态、分支选择、Diff Review、stage/unstage/discard、commit、push 和 worktree
界面属于 OpenTopia 内置工作台。这里的 `push` 是调用通用 Git remote，不表示调用 GitHub API。

当前 `git_workflow.rs` 已由 `local_git.rs` 包装为 provider-neutral `localGit.v1` Host Service，
而不是迁移到某个厂商插件。现有 `git_diff`、workspace diff、hunk review、turn undo、world
state Git 摘要仍保留在应用内。`localGit.v1` 当前覆盖 status、branches、remotes、stage、
unstage、discard、create/switch branch、commit、fetch/pull/push、compare 和 worktree
create/list/remove。discard 与 worktree remove 需要显式确认；所有 repository 都被限制在 thread
workspace，mutation 还要经过宿主 Policy 写权限裁决并产生工具事件。
宿主还以不含用户 ref/path/message 的稳定标签执行 command policy（如 `git push`）；策略返回
`Ask` 时 HTTP Host API fail-closed，不会把请求方传入的布尔值当作审批凭据。

### 4.2 插件提供：Code Host Connector

GitHub 官方插件及其他厂商插件提供以下远程服务能力：

| 通用概念 | GitHub | GitLab | 其他连接器示例 |
| --- | --- | --- | --- |
| 变更请求 | Pull Request | Merge Request | Bitbucket PR、Gitee PR、Gerrit Change |
| 工作项 | Issues | Issues | Azure Boards、Gitee Issues |
| 自动化 | Actions | Pipelines | Bitbucket Pipelines、Azure Pipelines |
| 审查 | Reviews/Comments | Approvals/Notes | 各厂商 review API |
| 发布 | Releases | Releases | 各厂商 release/package API |
| 账户与组织 | GitHub App/OAuth | OAuth/Token | 厂商自己的账户协议 |

连接器可以贡献：

- MCP tools 和配套 Skills。
- 仓库、PR/MR、Issue、CI 和 Review App views。
- 远程 URL matcher 和仓库身份解析器。
- 深链接、通知和远程状态 badge。
- 账户连接和 secret binding schema。
- “从当前 branch 创建 PR/MR”等组合工作流。

连接器不得：

- 替换 Local Git Service。
- 直接获得 `.git` 的不受限写权限。
- 自行绕过宿主创建 branch、commit、push 的审批和审计。
- 假设 `origin` 一定属于自身厂商。
- 因为远程账户已登录就自动获得本地写权限。

组合工作流应遵循：连接器通过受限 Host API 请求本地操作，Local Git Service 完成并记录
本地 mutation；连接器再调用远程 API。例如“提交并创建 GitHub PR”依次执行本地 commit、
通用 push、GitHub `create_pull_request`，三步分别产生工具事件和审批证据。

### 4.3 多连接器共存与选择

一个 workspace 可以同时安装多个代码托管连接器。选择规则如下：

1. Local Git Service 返回 remote name 和规范化 URL，不返回凭据。
2. 每个连接器声明支持的 URL matcher。
3. 精确 host/path 匹配优先于通配匹配。
4. 同一 remote 有多个匹配时，使用用户为该 remote 选择的默认连接器。
5. 没有匹配连接器时，本地 Git 功能仍全部可用，只隐藏远程厂商功能。
6. 不同 remote 可以分别绑定不同连接器，例如 `origin` 是 GitHub、`mirror` 是 Gitee。

当前已按 `(workspace_key, remote_name) -> connector_plugin_id + connector_id + account_binding_id`
持久化到 `scm_remote_bindings`，没有使用 workspace 级 `githubEnabled` 布尔值。Server 只允许绑定
当前激活且属于最佳匹配集合的 connector；没有 connector 时 Local Git 仍可用。

### 4.4 Git 插件命名规则

- 产品 UI 使用“Git”表示应用内置的本地版本控制能力。
- 产品 UI 使用“GitHub”“GitLab”等品牌名表示代码托管连接器。
- 插件分类使用 `scm_connector`，不使用 `git_provider`，避免和模型 Provider 混淆。
- 将来支持 Mercurial、Jujutsu 等其他本地 VCS 时，另设高信任
  `version_control_driver`，不要复用 Code Host Connector 协议。

## 5. 其他能力的归属决策

| 能力 | 架构归属/分发 | 应用开发者决定 | 管理员/用户可配置 | 当前状态 |
| --- | --- | --- | --- | --- |
| Agent loop、Turn/Event | Kernel，应用内置 | 协议和限制 | 不可替换 | 已完成 |
| Context/compaction | Kernel，应用内置 | 编译顺序、预算、安全项 | 已开放阈值和体验项 | 已完成，非插件化 |
| Policy/Approval/Sandbox | Kernel，应用内置 | 最终裁决和权限上限 | 在允许范围选择模式、作出审批 | 已完成，非插件化 |
| Workspace/File/Shell/Patch | Host Service，应用内置 | 路径、输出、超时和审计 | workspace、额外授权目录 | 已完成，非插件化 |
| Local Git | Host Service，应用内置 | 本地操作语义和安全策略 | remote、branch、commit/push 操作 | `localGit.v1` 首版操作集、安全门和持久化已完成 |
| GitHub/GitLab 等 | `scm_connector` 插件 | connector 协议、权限上限 | 安装、账户、per-remote 绑定、启停 | 协议底座；真实厂商 connector 未接入 |
| Skill loader | Kernel，应用内置 | roots、解析和注入上限 | 选择 Skill、作用域 | 已接入 scoped activation，保留兼容层 |
| Skill 创作 | Capability Plugin | 分发和写入限制 | 是否启用、创建目标 scope | 未完成抽取 |
| MCP host | Kernel/Host Service，应用内置 | 生命周期、沙箱、tool policy | server 启停和显式 env binding | 已接入 scoped activation，保留旧 binding |
| Spreadsheet | 默认安装 Bundled Plugin | 官方包、文件和资源上限 | 授权、启停、schema 参数 | 已完成包装和实际 tool/preview 接线；默认启用但未授权时不投影 |
| Browser Automation | 默认安装 Privileged Bundled Plugin | browser runtime、broker、域名策略上限 | 授权、启停、profile、下载和域名授权 | 已完成包装和 native tool activation；默认关闭，领域 runtime 仍为宿主内置 |
| Computer Use | 默认安装 Bundled Plugin + Trusted Driver | driver、窗口隔离、审批上限 | 授权、启停、窗口范围、即时审批 | 已完成包装和 native tool activation；默认关闭，driver 仍为宿主内置 |
| 文本/图片基础预览 | Preview Host，应用内置 | 安全 renderer 和大小上限 | 默认打开行为 | 已有既有实现 |
| XLSX rich preview | Spreadsheet Bundled Plugin | handler contract 和文件上限 | 启停 | 已完成实际接线 |
| 第三方 PDF/DOCX/PPTX rich preview | Preview Plugin | preview API、选择和隔离 | handler 选择、启停 | 受限 MCP v1 runtime 已完成；sidecar 暂不支持 |
| Context source 安全加载 | Host Service，应用内置 | canonical path、敏感文件和大小限制 | 选择附件 | 已有既有实现 |
| 格式提取/OCR/远程来源 | Context Loader Plugin | loader API 和数据上限 | handler、账户和参数 | workspace 文件的受限 MCP v1 runtime 已完成；远程来源另需 connector |
| 基础 Agent profiles | Kernel configuration，应用内置 | default/worker/explorer 基线 | 选择 profile | 已完成 |
| 领域 Agent profiles | 声明式插件 contribution | profile schema 和权限上限 | 启用和选择 | 已完成服务端加载与冲突约束；桌面选择体验待补 |
| App View | 受限插件 contribution | CSP、sandbox、channel 和大小上限 | 启停和打开视图 | Server 与 Desktop 最小承载、CSP/channel/cleanup 测试已完成 |
| Model Provider | Trusted Driver registry | driver 列表、secret 和传输协议 | 连接、模型、Endpoint、凭据 | 内置 registry 已完成；不支持普通插件动态注册 |
| Guardian/Reviewer | Kernel 默认实现；未来 Trusted Driver | fail-closed 和最终策略 | 可选择已批准策略 | 未抽取，按设计暂缓 |
| Artifact/Event/Store | Kernel/Host Service，应用内置 | schema、归属和审计 | 浏览、导出、产品允许的删除 | 已完成，非插件化 |

## 6. 插件协议

### 6.1 Manifest 兼容策略

继续支持现有 `.codex-plugin/plugin.json` 顶层字段：

- `name`
- `version`
- `description`
- `skills`
- `mcpServers`
- `apps`
- `interface`

OpenTopia 专有扩展放入命名空间字段 `opentopia`，避免破坏 Codex-compatible 插件：

```json
{
  "name": "example-scm",
  "version": "1.0.0",
  "skills": "./skills",
  "mcpServers": "./.mcp.json",
  "apps": "./apps.json",
  "opentopia": {
    "apiVersion": "1",
    "requires": {
      "hostCapabilities": ["localGit.read", "localGit.mutate.v1"]
    },
    "permissions": {
      "filesystem": ["workspace:read"],
      "network": ["api.example.com"],
      "secrets": ["account_token"],
      "desktop": []
    },
    "contributes": {
      "scmConnectors": ["./scm-connector.json"],
      "agentProfiles": ["./agents/reviewer.toml"],
      "previewers": []
    },
    "configuration": {
      "schema": "./configuration.schema.json"
    }
  }
}
```

`trust`、`official`、`signatureVerified` 等字段不由插件自报，而由安装记录根据来源生成。

### 6.2 Contribution 类型

Manifest v1 当前已识别并规范化：

- `skills`
- `mcpServers`
- `apps`
- `nativeTools`
- `configuration`
- `agentProfiles`
- `scmConnectors`
- `previewers`
- `contextLoaders`

尚未进入 manifest v1 的普通 contribution：

- `artifactHandlers`

高信任 driver 不通过普通 contribution 自动注册，使用独立 registry：

- `providerDrivers`
- `computerDrivers`
- `executionEnvironmentDrivers`
- `reviewerDrivers`
- `versionControlDrivers`

“被 manifest 识别”不等于“运行时完整”。当前 Skills、MCP、Bundled native tools、Spreadsheet
preview 和 Agent profiles 已接入 scoped activation；App View 有服务端宿主；第三方 Previewer 和
Context Loader 只有注册、匹配和选择协议，尚未执行其 `runtime`。

### 6.3 Capability Registry

插件发现只产生 descriptor，不应立即启动进程。Capability Registry 在 activation 后生成：

- contribution 的全局唯一 ID：`<plugin-id>/<local-id>`。
- contribution 类型和版本。
- 安装来源和信任等级。
- global/workspace/thread activation 状态。
- 所需和已授予权限。
- runtime health、最后错误和重启计数。
- 与 MCP server、Skill、App view 的来源关联。

Agent 的工具目录、Skill 目录和桌面 App 目录都从同一份 activation snapshot 投影，避免
当前 Skill 选择和 MCP thread enablement 各自维护一套插件状态。

**当前状态：已完成核心 registry，运行时仍有兼容层。** Registry 已实现稳定 ID、manifest
版本校验、host capability 检查、permission 检查和独占 contribution 冲突检测。Server 已返回
thread capability snapshot，并用 scoped activation 驱动 Bundled native tools、MCP、Skills、
Spreadsheet preview 和 Agent profiles。旧 thread-only plugin activation 与 MCP binding 仍在无新
activation 记录时作为 fallback。

当前还有一个必须修正的语义差异：snapshot 会把缺少 manifest permission grant 的 contribution
标记为 unavailable，但 Bundled native tool 的实际执行仍主要依赖现有 Policy/Approval/Sandbox，
没有统一以新 grant 结果作为启停条件。不能把 capability API 的 unavailable 状态视为已经完全
阻断运行时调用。

### 6.4 Activation 生命周期

生命周期固定为：

1. Discover：有界扫描 manifest，不执行插件代码。
2. Inspect：校验路径、schema、兼容性、contributions 和权限申请。
3. Install：复制到 staging，校验后原子安装。
4. Configure：收集 schema 值和 secret binding，不把 secret 返回 renderer。
5. Grant：用户确认可选权限，平台拒绝越界权限。
6. Activate：按 global/workspace/thread 创建 capability snapshot。
7. Start：仅在首次需要时启动 MCP/sidecar/App session。
8. Health：记录 ready/degraded/error，不因单插件失败阻止 Kernel 启动。
9. Deactivate：停止新调用，等待或取消在途调用，释放进程和 UI session。
10. Upgrade/Uninstall：先 deactivate，再迁移配置或清理安装目录；审计历史保留来源快照。

### 6.5 运行隔离

- 普通工具插件优先使用 MCP/JSON-RPC 进程边界。
- 不使用 Rust dynamic library 作为公共 ABI。
- 插件不接收 `ToolContext`；宿主只提供有版本的 capability handles。
- 插件进程使用现有 `ExecutionEnvironment` 和 sandbox plan 启动。
- App view 使用 sandbox iframe 或声明式 UI，禁止 Node integration。
- Secret 通过 opaque binding ID 注入目标进程，renderer 和模型都不能读取明文。
- 所有插件工具调用继续生成标准 ToolCall/ToolResult/Event，并经过 Policy。
- 插件输出继续使用统一大小、多模态和敏感信息限制。

## 7. 数据模型与 API

### 7.1 持久化模型

当前已按现有 SQLite migration 风格增加：

- `plugin_activations`
  - plugin ID、scope type、scope ID、enabled、updated at。
- `plugin_settings`
  - plugin ID、scope、key、JSON value；不存 secret 明文。
- `plugin_secret_bindings`
  - plugin ID、setting key、safe-storage reference、scope、metadata。
- `plugin_permission_grants`
  - plugin ID、scope、permission、constraint、status、granted at。
- `plugin_runtime_health`
  - contribution ID、status、last error、last checked at、restart count。
- `scm_remote_bindings`
  - workspace key、remote name、connector plugin ID、connector ID、account binding ID。

现有 `mcp_servers.plugin_id` 和 `plugin_server_name` 保留，迁移后成为 contribution 来源索引。

尚未建立统一 `plugin_installations` 表。普通插件仍由当前安装目录和 manifest 发现，Bundled Plugin
由宿主 receipt 和包内容逐字节校验来源；如果后续需要市场、签名升级历史或安装审计，再增加统一
installation ledger，不能用 manifest 自报字段代替。

### 7.2 Server API

当前已实现：

- `GET /api/plugins/:plugin_id`
- `PUT /api/plugins/:plugin_id/activation`
- `GET/PATCH /api/plugins/:plugin_id/settings`
- `GET/PUT /api/plugins/:plugin_id/permissions`
- `GET /api/plugins/:plugin_id/contributions`
- `GET /api/plugins/:plugin_id/health`
- `GET /api/threads/:thread_id/capabilities`
- `GET/PUT /api/threads/:thread_id/scm/remotes/:remote_name/connector`
- `POST /api/threads/:thread_id/local-git/v1`
- `GET /api/provider/drivers`
- `GET /api/threads/:thread_id/contribution-hosts`
- `GET /api/threads/:thread_id/preview-handler`
- `GET /api/threads/:thread_id/context-loader`
- `POST /api/threads/:thread_id/plugin-app-sessions`
- `POST /api/threads/:thread_id/plugin-app-sessions/:session_id/messages`
- `GET /api/threads/:thread_id/plugin-app-sessions/:session_id/content`
- `DELETE /api/threads/:thread_id/plugin-app-sessions/:session_id`

所有设置写入必须由后端按插件 schema 校验。Renderer 不负责权限裁决，也不能提交 schema
之外的任意环境变量。

### 7.3 Local Git Service 接口

现有 Git API 已开始收敛为版本化 service，连接器不直接拼接 shell。当前 `localGit.v1` 支持：

```text
localGit.v1.status(repository)
localGit.v1.branches(repository)
localGit.v1.remotes(repository)
localGit.v1.createBranch(repository, request)
localGit.v1.switchBranch(repository, request)
localGit.v1.commit(repository, request)
localGit.v1.push(repository, request)
localGit.v1.compare(repository, request)
localGit.v1.createWorktree(repository, request)
```

给普通连接器的 host contract 默认只开放读取 handle，mutation handle 包含 grant ID。接口参数
全部结构化，不接受任意 Git argv；Server 还会把 repository 限制在 thread workspace 中。

仍需补入 `localGit.v1` 的能力包括 diff/hunk、stage/unstage/discard、fetch/pull、worktree
list/remove。现有应用内 Git 工作流可继续提供这些能力，但 connector 不应绕过版本化 Host API
直接调用任意 Git 命令。

## 8. 迁移策略

### Phase 0：冻结边界

**状态：已完成。**

- 将本文的 Kernel、Host Service、Bundled Plugin、Standard Plugin、Trusted Driver 分类落实为类型。
- 给 manifest 增加 `opentopia.apiVersion`、permissions 和 contributions 解析，但保持旧插件兼容。
- 建立统一 capability snapshot，暂不移动现有功能。

### Phase 1：完成插件底座

**状态：部分完成。** Activation、settings、permission grants、health、secret binding、Capability
Registry 和 Server/Desktop 控制面已落地。Skills、MCP 和 native tools 已接入 scoped activation，
但仍有旧 thread activation/MCP binding fallback；App bridge 服务端已完成，桌面最小承载在本轮
实现并等待集成验证。

- 实现 activation、settings、permission grants、health 和 secret binding。
- 验收受限 App bridge 的 Server/Desktop 生命周期、消息边界和清理行为。
- Skill 与 MCP 从 capability snapshot 派生，同时保留旧 API 兼容层。

### Phase 2：验证普通插件

**状态：已完成首个 Bundled Plugin。** Spreadsheet 已默认安装，现有 native tool 和 XLSX
preview 由该插件 activation 控制；领域实现仍保留为宿主内置 runtime adapter。

- 先将 Spreadsheet 迁移为官方 Bundled Plugin。
- rich XLSX preview 与 Spreadsheet tool 使用同一个插件来源。
- 保留基础 Preview Host、路径安全和文件大小限制在应用内。

### Phase 3：验证高权限插件

**状态：部分完成。** Browser Automation 和 Computer Use 已作为默认安装 Bundled Plugin，
宿主拥有其 Privileged/Trusted Driver 信任元数据，native tool activation 已接线。独立可扩展的
Computer Driver Registry、统一 permission grant 执行门和完整桌面端插件 App UI 仍未完成。

- 将 Browser Automation 包装成 Privileged Bundled Plugin。
- 将 Computer backend 放入 Trusted Driver registry，工具和 App UI 由官方插件贡献。
- 验证域名、窗口、下载、截图、多模态和 approval continuation 不发生回归。

### Phase 4：代码托管连接器

**状态：协议底座完成，厂商插件未接入。** Local Git、remote matcher、冲突选择、per-remote
binding、opaque account binding 和部分成功结果类型已实现；尚无实际 GitHub/GitLab connector
adapter，也没有调用 connector runtime 创建 PR/MR 的端到端工作流。

- 保持 Local Git Service 和内置 Git 工作台不变。
- 接入官方 GitHub 插件作为第一个 `scm_connector`。
- 用最小测试连接器或 GitLab 插件验证多厂商协议不是为 GitHub 特制。
- 实现 per-remote connector/account binding。

### Phase 5：高信任驱动

**状态：Provider 部分已完成。** `ProviderDriverRegistry` 已接管内置 Mock、OpenAI-compatible、
OpenAI Responses、Anthropic 和 Codex App Server 构造，并通过只读 API 暴露 descriptor。注册入口
保持 crate 内私有；Guardian、ExecutionEnvironment、Computer 等独立 driver registry 尚未实现。

- 保持 `ProviderKind` 兼容设置，通过 `ProviderDriverRegistry` 统一构造内置 driver。
- 首期 driver 仍由应用内置，不开放普通目录自动加载。
- Guardian reviewer 和 ExecutionEnvironment driver 等到出现第二个真实实现后再迁移。

### Phase 6：其余声明式贡献

**状态：分项进行中。** Agent profiles 已完成服务端声明式加载；Preview/Context handler 已完成
注册选择协议；App View 已完成服务端 sandbox/session/content/message host，桌面最小承载本轮
实现、待验证。第三方 handler runtime、artifact handler 和 Skill/Plugin 创作能力抽取仍未完成。

- Agent profiles、context loaders、artifact handlers、DOCX/PPTX/PDF rich preview。
- 将 Skill/Plugin 创作能力迁移为官方系统插件。

## 9. 多 Agent 实现拆分与状态

以下工作包保留为文件所有权和后续任务边界。继续并行实现时仍要避免多个 Agent 同时修改
`main.rs`、`store.rs` 或 `App.tsx`。

### 批次 A：协议和存储

**A1 Manifest/Capability Contract：已完成**

- 所有权：`crates/opentopia-core/src/plugins.rs`，新增独立 capability 模块。
- 产出：版本化 manifest、permissions、contribution descriptor、兼容性测试。
- 不负责：Server routes、桌面 UI。

**A2 Plugin Persistence：已完成控制面表；统一 installation ledger 未完成**

- 所有权：`crates/opentopia-core/src/store.rs` 中新增的插件表和 store API。
- 已产出 activation、settings、grant、health、SCM binding migrations 和测试；安装仍由目录发现与
  Bundled receipt 管理。
- 依赖：与 A1 先约定序列化类型，避免直接引用未稳定结构。

**A3 Local Git Boundary：已完成基础 `localGit.v1`；操作集合待扩充**

- 所有权：`git_workflow.rs` 及新增 `local_git` service 模块。
- 产出：明确的 provider-neutral service、remote descriptor、结构化 mutation API 和测试。
- 必须保持：现有 Git API 行为、Diff Review、world state 和 turn undo。

### 批次 B：运行时和 API

**B1 Capability Runtime：已完成首版，保留旧兼容层**

- 产出：activation snapshot、lazy start、deactivate、health、MCP/Skill 来源统一。
- 主要文件：新增 runtime 模块，最小改动 `agent.rs`、`mcp_host.rs`、`skills.rs`。
- 依赖：A1、A2。

**B2 Server Control Plane：已完成**

- 产出：插件 settings/activation/permissions/health/capabilities API。
- 主要文件：建议把 routes 从 `main.rs` 拆入 `plugins_api.rs`，避免继续放大 `main.rs`。
- 依赖：A1、A2、B1。

**B3 SCM Connector Host：协议底座已完成**

- 产出：remote matcher、per-remote binding、connector read/mutation handle 和组合事件。
- 不实现 GitHub API；只实现厂商无关 host contract。
- 依赖：A3、B1、B2。

### 批次 C：桌面和首个插件

**C1 Plugin Settings UI：已完成首版**

- 产出：schema-driven settings、权限查看/撤销、scope activation、health 状态。
- 开始修改 UI 前必须遵守仓库 `AGENTS.md`，读取设计系统文档并运行设计检查与类型检查。
- 依赖：B2。

**C2 App View Host：Server 与 Desktop 最小承载已完成并通过回归测试**

- 产出：受限 App view 注册、路由、生命周期和消息协议。
- 不允许插件任意导入 renderer 代码或获得 Electron preload 全量接口。
- 依赖：A1、B1。

**C3 Spreadsheet Extraction：已完成 Bundled Plugin 包装和接线**

- 产出：官方 Spreadsheet Bundled Plugin，功能与现有 tool/preview 等价。
- 在迁移验收完成前保留兼容 adapter，不一次删除旧实现。
- 依赖：B1、C2。

### 批次 D：GitHub 与高权限能力

**D1 GitHub Connector Integration：未完成**

- 使用官方 GitHub 插件已有 Skills/MCP/App 能力。
- 增加 remote matcher 和 account binding adapter，不复制 GitHub 插件的业务逻辑。
- 验证没有 GitHub 插件时 Local Git UI 和 Agent 工具完全可用。
- 依赖：B3、C2。

**D2 Second Connector Conformance Test：未完成**

- 使用 GitLab mock connector 或最小测试连接器验证协议。
- 必测：同 workspace 多 remote、同 remote 多 matcher、用户默认选择和解绑。
- 依赖：B3。

**D3 Browser/Computer Packaging：包装与 scoped permission projection 已完成**

- 先包装现有 runtime，不在同一任务中重写浏览器或桌面控制实现。
- 验证 approval、sandbox、截图、多模态、下载和 session 恢复。
- 依赖：B1、C2、权限与 Trusted Driver registry。

### 批次 E：Provider 与长尾贡献

**E1 ProviderDriverRegistry：已完成内置 driver registry**

- 已通过 registry 替换 Server 的重复 provider 构造分支，并保持设置兼容。
- 普通插件目录不能自动注册 Provider driver。
- 依赖：Trusted Driver 基础类型稳定。

**E2 Preview/Source Contributions：注册选择与受限 MCP v1 runtime 已完成**

- 产出：MIME/extension handler registry、优先级、用户默认选择和 fallback。
- 保持 canonical path、安全上限、artifact ownership 在 Host Service。
- runtime 只允许调用同一插件声明且已激活的 MCP server/tool；sidecar executor 暂不开放。

**E3 Declarative Agent Profiles：服务端已完成**

- 产出：插件 profile discovery、来源追踪、冲突规则和 tool allow/deny 上限。
- 插件 profile 不能放宽父 Agent 或平台权限。

## 10. 验收标准

### 10.1 通用插件

- **已完成：** 旧的仅 Skills/MCP 插件仍可发现、安装和启用。
- **已完成：** `apps` 可得到受限、可停止的服务端 session；Desktop 使用无 Node 权限的 sandbox
  iframe、manifest channel 白名单和 start/stop cleanup，空 channel 列表 fail-closed。
- **已完成首版：** scoped activation 已驱动 native tools、MCP、Skills、Spreadsheet preview 和
  Agent profiles；旧 activation/MCP fallback 尚未移除。
- **已完成：** Renderer 只处理 opaque secret binding ID，不读取 secret 明文。
- **未完全验证：** 卸载后历史 ToolCall/Event 来源快照、通用 sidecar 健康隔离和崩溃恢复。
- **已完成首版：** 新 permission grant、scope activation 和 conflict 共同生成 capability snapshot；
  native/MCP/Skill/Profile/App/Preview/Context/SCM 的实际投影均使用其 active contributions。

### 10.2 Git 与代码托管连接器

- **已完成：** 未安装任何 connector 时，本地 Git 工作台和 Agent 本地 Git 能力不依赖 GitHub。
- **已完成：** Local Git repository 被限制在 thread workspace；connector host 使用结构化 handle。
- **已完成：** 同一 workspace 的不同 remote 可持久化不同 connector/account binding。
- **已完成协议：** matcher、冲突、best-match、解绑和部分成功结果类型。
- **已完成：** `localGit.v1` 覆盖 stage/unstage/discard、fetch/pull、worktree list/remove；高风险
  discard/remove 需要显式确认，mutation 经过路径和 Policy 写权限检查并记录工具事件。
- **未完成：** 官方 GitHub connector 与第二个 GitLab/mock connector conformance test。
- **未完成：** “commit + push + create PR/MR”实际调用 connector runtime、生成分阶段事件和 UI 呈现。

### 10.3 配置责任

- **已完成协议：** 平台权限上限不能被 manifest、workspace 或 thread 设置扩大；窄 activation
  采用单调 AND 语义。
- **已完成：** secret 字段不进入普通 JSON settings，控制面只存 opaque binding ID。
- **已完成首版 UI：** 用户可查看权限申请、grant/revoke、settings、health 和 contributions。
- **已完成首版：** activation snapshot 统一驱动 App/Preview/Context 等 contribution 的运行时可见性；
  被撤权、停用或发生冲突的 contribution 不再可调用，现有 App session 也会失效并停止。
- **部分完成：** Bundled Plugin 的官方/Privileged/Trusted Driver 信任由宿主验证；独立管理员
  RBAC、签名第三方来源和统一 installation ledger 尚未完成。

## 11. 非目标

本轮设计不要求：

- 建立公网插件市场、计费和自动发布系统。
- 支持任意 Rust/Node 代码在 OpenTopia 主进程或 renderer 内加载。
- 让插件替换 SessionStore、Agent loop、Policy 或 Sandbox。
- 把现有 Local Git 工作台整体迁移到 GitHub 插件。
- 首期开放第三方 Provider、Computer 或 Execution driver 自动加载。
- 在插件底座尚未稳定时同时重写 Browser、Computer 或 Spreadsheet 的领域实现。

## 12. 当前代码对应关系

截至 2026-08-01，代码映射如下。

### 12.1 已完成

- `capabilities.rs`：Manifest API v1、Codex-compatible contribution normalization、稳定
  `<plugin-id>/<local-id>`、Capability Registry、activation snapshot、host capability/permission/
  conflict 检查。
- `bundled_plugins/` 与 `bundled-plugins/`：Spreadsheet、Browser Automation、Computer Use 三个
  包均随 Server 启动默认安装；Spreadsheet 默认启用，Browser Automation 和 Computer Use 默认
  关闭；宿主 receipt 和包内容校验决定 official trust，稳定宿主 ID 不包含安装绝对路径。
- `plugins.rs`：统一发现 Bundled、User、Workspace 和 Codex cache 插件，解析 OpenTopia v1
  manifest，同时保持 Skills/MCP/Apps 兼容字段。
- `plugin_control.rs`、`plugins_api.rs`：global/workspace/thread activation、schema settings、opaque
  secret binding、permission grant/revoke、contribution、health 和 thread capability snapshot。
- `PluginControlPanel.tsx`：桌面首版插件控制面，覆盖 activation、settings、permissions、health
  和 contributions。
- `main.rs`：同一份 scoped capability snapshot 已驱动 Bundled native tools、插件 MCP tool
  projection、Skills、Spreadsheet preview、App/Profile/SCM contributions；普通非插件 MCP server
  仍保留旧 thread binding 兼容路径。
- `local_git.rs`、`scm_connector.rs`、`scm_api.rs`：provider-neutral `localGit.v1`、remote URL
  normalization、完整首版结构化本地操作、mutation Policy gate、connector matcher/conflict、
  per-remote binding、opaque account binding 和 `commit -> push -> change request` 分阶段结果类型。
- `provider.rs`：内置 `ProviderDriverRegistry`，注册入口不对普通插件开放；Server 提供
  `GET /api/provider/drivers`。
- `agent_profiles.rs`：只从激活插件包内 TOML/JSON 加载声明式 profile，禁止覆盖 built-in/
  workspace profile，冲突时忽略，tool allow 取交集、deny 累加、sandbox 只能收紧。

### 12.2 Host runtime 已完成首版，厂商插件与通用 sidecar 尚未闭环

- `contribution_hosts.rs`、`contributions_api.rs` 已实现 Preview/Context MIME/extension 选择、
  priority/tie conflict、版本化 MCP v1 调用、输入/参数/输出上限、health 与工具事件；同时实现
  App View 包内相对路径、CSP sandbox、session lifecycle 和有界 message。
- App View 当前可以在 Server 创建 session、读取受限 HTML、发送允许的 message 和停止 session；
  Desktop 已实现最小 sandbox iframe、严格 CSP 注入、manifest channel bridge 和 session cleanup，
  并有 start/message/stop、空 channel fail-closed 和 renderer 边界测试。
- 第三方 Previewer/Context Loader 可通过 `mcp.v1:<server>/<tool>` 调用同一激活插件声明的 MCP
  server/tool；来源必须是 workspace 内 canonical file，需 `workspace:read` grant，并继续经过
  Policy 与 MCP tool policy。通用 sidecar runtime 明确 fail-closed，留待独立 supervisor。
- SCM connector 已能声明 matcher、被选择和绑定，但尚没有真实 GitHub/GitLab plugin adapter
  执行远程 API。部分成功类型存在，不等于组合工作流已经端到端可用。
- Plugin health 已有表、API 和 UI，但缺少统一 sidecar supervisor 自动写入 ready/degraded/error、
  重启计数和 deactivate 清理的完整生命周期。

### 12.3 真实剩余缺口

1. 接入官方 GitHub SCM connector，并用 GitLab/mock 第二实现做多厂商 conformance test。
2. 实现 connector 组合工作流与分阶段 Event/Approval/UI，验证 push 成功而 PR/MR 失败的部分成功。
3. 为 App View 补齐失败、重载、切换 thread、卸载插件和真实 Electron 导航的跨平台端到端测试。
4. 若第三方插件确有后台进程需求，再实现通用 sidecar supervisor、签名/信任要求、资源隔离、
   崩溃恢复与 restart budget；当前 sidecar runtime 不开放。
5. 按产品 UI 需要决定是否把更多 diff/hunk 操作加入 `localGit.v1`；现有 Diff Review 与 turn undo
   继续使用应用内置 Host/Kernel 实现，不依赖 SCM connector。
6. 增加独立部署管理员/Workspace Owner 的鉴权和 RBAC；global/workspace/thread scope 本身不是
   actor authorization。
7. 视发布和市场需求增加统一 plugin installation ledger、签名第三方来源和升级审计。
8. 后续再抽取 Computer、ExecutionEnvironment、Reviewer 等 Trusted Driver registry；普通插件目录
   仍不得自动注册高信任 driver。
9. 完成其他平台打包环境和真实 Electron 验收，再删除旧 thread activation/MCP
   binding 兼容层。

### 12.4 本轮验收记录

- Windows 开发环境已通过 `cargo check --workspace`、Core 441 项测试、Server 69 项测试、Desktop
  144 项测试、TypeScript typecheck、设计系统检查、Rustfmt、Prettier 和 `git diff --check`。
- 独立端口与独立 SQLite/Bundled root 的真实 HTTP 验收已验证：三个 Bundled 包默认安装，
  Browser/Computer 默认关闭，Spreadsheet 未授权时 unavailable、thread 授权后 NativeTool 与
  Previewer 同时 active，Provider Driver registry 返回五个 built-in driver。
- 同一 HTTP 实例已验证无 SCM connector 时 `origin` 为 `unmatched` 但 `localGit.v1 status` 正常，
  且对应 `ToolCallStarted`/`ToolCallFinished` 两条审计事件持久化成功。

继续实现时应通过协议测试和迁移测试锁定上述边界；不要在抽取插件的同时改变现有领域行为或
进行大范围 UI 重写。
