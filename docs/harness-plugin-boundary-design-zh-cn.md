# OpenTopia Harness 内置能力与插件边界设计

状态：设计草案  
日期：2026-07-31  
目标读者：OpenTopia 维护者、插件作者、后续并行实现 Agent

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

### 3.1 平台维护者配置

以下内容由 OpenTopia 维护者定义，最终用户不能修改：

- 插件 API 和 manifest schema 版本。
- 支持的 contribution 类型及其输入输出协议。
- 插件信任等级、签名来源和允许的运行方式。
- 文件系统、网络、桌面、进程和 secret 的权限上限。
- 沙箱、审批、审计、超时、输出上限和资源配额。
- App view 的隔离方式及允许的宿主 UI API。
- 哪些插件随安装包分发，哪些默认可用。
- Provider、Computer、Execution 和 Reviewer driver 的允许列表。
- Kernel prompt、安全 prompt 和不可覆盖的行为约束。
- Local Git Service 的语义、路径保护和 mutation policy。

### 3.2 插件作者声明

插件作者在 manifest 和配置 schema 中声明：

- 插件元数据、版本和最低宿主 API 版本。
- Skills、MCP servers、Apps、Agent profiles 等 contributions。
- 所需权限和所需 Host Service 能力。
- 用户可配置字段、默认值、校验规则和 secret 字段标记。
- activation 条件、健康检查和卸载清理声明。
- 代码托管连接器支持的远程 URL 类型和功能矩阵。

插件只能“申请”权限，不能给自己授权；manifest 中也不允许声明自身信任等级。

### 3.3 最终用户配置

用户可以配置：

- 安装、卸载、启用和禁用插件。
- 插件在全局、Workspace 或 Thread 范围内是否生效。
- 选择本轮使用的 Skills、Agent profiles 和默认预览处理器。
- 插件 schema 明确开放的普通参数。
- Provider 连接、模型、Endpoint 和凭据值。
- Browser profile、下载目录、域名授权和 Computer 窗口范围。
- MCP server 的启停、超时和显式环境变量绑定。
- 代码托管账户、远程仓库映射和默认连接器。
- 可选权限授权，以及撤销已经授予的权限。
- 自动更新、手动更新或版本锁定策略。

用户不能配置：

- 绕过 Policy、Approval、Sandbox 或审计。
- 让普通插件读取任意目录、其他插件配置或 Provider 密钥。
- 把普通插件升级为 Trusted Driver。
- 修改或删除历史审批、工具事件和审计记录。
- 用插件 prompt 覆盖 Kernel 安全指令。
- 允许 App view 在 renderer 中执行不受限 Node/Electron API。

### 3.4 配置优先级

运行时采用以下优先级，前者是约束，后者只能在约束内细化：

1. 平台不可变安全约束。
2. 安装来源对应的信任策略。
3. 插件 manifest 的能力和默认值。
4. 用户全局设置。
5. Workspace 设置。
6. Thread activation 和本轮选择。
7. 每次高风险调用的即时审批结果。

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

现有 `git_workflow.rs` 应演进为 Local Git Service 的实现，而不是整体迁移到插件。现有
`git_diff`、workspace diff、hunk review、turn undo、world state Git 摘要同样保留在应用内。

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

建议持久化键为 `(workspace_key, remote_name) -> connector_plugin_id + account_binding_id`，
而不是给整个 workspace 只保存一个 `githubEnabled` 布尔值。

### 4.4 Git 插件命名规则

- 产品 UI 使用“Git”表示应用内置的本地版本控制能力。
- 产品 UI 使用“GitHub”“GitLab”等品牌名表示代码托管连接器。
- 插件分类使用 `scm_connector`，不使用 `git_provider`，避免和模型 Provider 混淆。
- 将来支持 Mercurial、Jujutsu 等其他本地 VCS 时，另设高信任
  `version_control_driver`，不要复用 Code Host Connector 协议。

## 5. 其他能力的归属决策

| 能力 | 架构归属 | 分发方式 | 平台维护者决定 | 用户可配置 |
| --- | --- | --- | --- | --- |
| Agent loop、Turn/Event | Kernel | 应用内置 | 协议和限制 | 不可替换 |
| Context/compaction | Kernel | 应用内置 | 编译顺序、预算、安全项 | 阈值和已开放体验项 |
| Policy/Approval/Sandbox | Kernel | 应用内置 | 最终裁决和权限上限 | 在允许范围选择模式、作出审批 |
| Workspace/File/Shell/Patch | Host Service | 应用内置 | 路径、输出、超时和审计 | workspace、额外授权目录 |
| Local Git | Host Service | 应用内置 | 本地操作语义和安全策略 | remote、branch、commit/push 操作 |
| GitHub/GitLab 等 | Code Host Connector | 普通或官方插件 | connector 协议、权限上限 | 安装、账户、remote 绑定、启停 |
| Skill loader | Kernel | 应用内置 | roots、解析和注入上限 | 选择 Skill、作用域 |
| Skill 创作 | Capability Plugin | 官方系统插件 | 分发和写入限制 | 是否启用、创建目标 scope |
| MCP host | Kernel/Host Service | 应用内置 | 生命周期、沙箱、tool policy | server 启停和显式 env binding |
| Spreadsheet | Capability Plugin | 官方捆绑插件 | 插件版本和文件限制上限 | 启停、输出目录、格式选项 |
| Browser Automation | Privileged Bundled Plugin | 官方高信任插件 | backend、broker、域名策略上限 | 启停、profile、下载和域名授权 |
| Computer Use | Trusted Driver + Plugin UI | 官方高信任插件 | 平台 backend、窗口隔离、审批 | 启停、窗口范围、即时审批 |
| 文本/图片基础预览 | Preview Host | 应用内置 | 安全 renderer 和大小上限 | 默认打开行为 |
| PDF/XLSX/DOCX/PPTX rich preview | Preview Plugin | 官方或第三方插件 | preview API 和隔离 | handler 选择、启停 |
| Context source 安全加载 | Host Service | 应用内置 | canonical path、敏感文件和大小限制 | 选择附件 |
| 格式提取/OCR/远程来源 | Source Plugin | 插件 | loader API 和数据上限 | handler、账户和参数 |
| 基础 Agent profiles | Kernel configuration | 应用内置 | default/worker/explorer 基线 | 选择 profile |
| 领域 Agent profiles | Declarative contribution | 插件 | profile schema 和权限上限 | 启用、选择模型和公开参数 |
| Model Provider | Trusted Driver | 内置或签名驱动 | driver 列表、secret 和传输协议 | 连接、模型、Endpoint、凭据 |
| Guardian/Reviewer | Kernel 默认实现；以后可用 Trusted Driver | 应用内置优先 | fail-closed 和最终策略 | 可选择已批准策略，不能关闭强制检查 |
| Artifact/Event/Store | Kernel/Host Service | 应用内置 | schema、归属和审计 | 浏览、导出、产品允许的删除 |

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

第一阶段支持：

- `skills`
- `mcpServers`
- `apps`
- `configuration`
- `agentProfiles`
- `scmConnectors`

第二阶段支持：

- `previewers`
- `contextLoaders`
- `artifactHandlers`

高信任 driver 不通过普通 contribution 自动注册，使用独立 registry：

- `providerDrivers`
- `computerDrivers`
- `executionEnvironmentDrivers`
- `reviewerDrivers`
- `versionControlDrivers`

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

## 7. 数据模型与 API 建议

### 7.1 持久化模型

建议增加下列概念表；具体 SQL 可以按现有 SQLite migration 风格实现：

- `plugin_installations`
  - plugin ID、version、path、source、trust class、manifest hash、installed at。
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
  - workspace key、remote name、connector plugin ID、account binding ID。

现有 `mcp_servers.plugin_id` 和 `plugin_server_name` 保留，迁移后成为 contribution 来源索引。

### 7.2 Server API

在现有插件 API 基础上补充：

- `GET /api/plugins/:plugin_id`
- `PUT /api/plugins/:plugin_id/activation`
- `GET/PATCH /api/plugins/:plugin_id/settings`
- `GET/PUT /api/plugins/:plugin_id/permissions`
- `GET /api/plugins/:plugin_id/contributions`
- `GET /api/plugins/:plugin_id/health`
- `GET /api/threads/:thread_id/capabilities`
- `GET/PUT /api/threads/:thread_id/scm/remotes/:remote_name/connector`

所有设置写入必须由后端按插件 schema 校验。Renderer 不负责权限裁决，也不能提交 schema
之外的任意环境变量。

### 7.3 Local Git Service 接口

建议将现有 Git API 收敛为版本化 service，而不是由连接器调用 shell：

```text
localGit.v1.status(repository)
localGit.v1.diff(repository, scope)
localGit.v1.stage(repository, paths_or_hunks)
localGit.v1.unstage(repository, paths_or_hunks)
localGit.v1.discard(repository, paths_or_hunks)
localGit.v1.branches(repository)
localGit.v1.createBranch(repository, request)
localGit.v1.switchBranch(repository, request)
localGit.v1.commit(repository, request)
localGit.v1.remotes(repository)
localGit.v1.fetch(repository, request)
localGit.v1.pull(repository, request)
localGit.v1.push(repository, request)
localGit.v1.worktrees(repository)
localGit.v1.createWorktree(repository, request)
```

给普通连接器的 handle 默认只开放读取接口。Mutation handle 必须由插件声明、用户授权，并且
每次调用仍进入宿主 Policy/Approval。接口参数全部结构化，不接受任意 Git argv。

## 8. 迁移策略

### Phase 0：冻结边界

- 将本文的 Kernel、Host Service、Bundled Plugin、Standard Plugin、Trusted Driver 分类落实为类型。
- 给 manifest 增加 `opentopia.apiVersion`、permissions 和 contributions 解析，但保持旧插件兼容。
- 建立统一 capability snapshot，暂不移动现有功能。

### Phase 1：完成插件底座

- 实现 activation、settings、permission grants、health 和 secret binding。
- 实现受限 App bridge；当前只检测 `apps` 而不支持运行的状态必须结束。
- Skill 与 MCP 从 capability snapshot 派生，同时保留旧 API 兼容层。

### Phase 2：验证普通插件

- 先将 Spreadsheet 迁移为官方 Bundled Plugin。
- rich XLSX preview 与 Spreadsheet tool 使用同一个插件来源。
- 保留基础 Preview Host、路径安全和文件大小限制在应用内。

### Phase 3：验证高权限插件

- 将 Browser Automation 包装成 Privileged Bundled Plugin。
- 将 Computer backend 放入 Trusted Driver registry，工具和 App UI 由官方插件贡献。
- 验证域名、窗口、下载、截图、多模态和 approval continuation 不发生回归。

### Phase 4：代码托管连接器

- 保持 Local Git Service 和内置 Git 工作台不变。
- 接入官方 GitHub 插件作为第一个 `scm_connector`。
- 用最小测试连接器或 GitLab 插件验证多厂商协议不是为 GitHub 特制。
- 实现 per-remote connector/account binding。

### Phase 5：高信任驱动

- 把现有 `ProviderKind` 封闭构造迁移到 `ProviderDriverRegistry`。
- 首期 driver 仍由应用内置，不开放普通目录自动加载。
- Guardian reviewer 和 ExecutionEnvironment driver 等到出现第二个真实实现后再迁移。

### Phase 6：其余声明式贡献

- Agent profiles、context loaders、artifact handlers、DOCX/PPTX/PDF rich preview。
- 将 Skill/Plugin 创作能力迁移为官方系统插件。

## 9. 多 Agent 实现拆分

以下工作包按依赖顺序执行。标记同一批次的工作仍应约定文件所有权，避免多个 Agent 同时修改
`main.rs`、`store.rs` 或 `App.tsx`。

### 批次 A：协议和存储

**A1 Manifest/Capability Contract**

- 所有权：`crates/opentopia-core/src/plugins.rs`，新增独立 capability 模块。
- 产出：版本化 manifest、permissions、contribution descriptor、兼容性测试。
- 不负责：Server routes、桌面 UI。

**A2 Plugin Persistence**

- 所有权：`crates/opentopia-core/src/store.rs` 中新增的插件表和 store API。
- 产出：installation、activation、settings、grant、health、SCM binding migrations 和测试。
- 依赖：与 A1 先约定序列化类型，避免直接引用未稳定结构。

**A3 Local Git Boundary**

- 所有权：`git_workflow.rs` 及新增 `local_git` service 模块。
- 产出：明确的 provider-neutral service、remote descriptor、结构化 mutation API 和测试。
- 必须保持：现有 Git API 行为、Diff Review、world state 和 turn undo。

### 批次 B：运行时和 API

**B1 Capability Runtime**

- 产出：activation snapshot、lazy start、deactivate、health、MCP/Skill 来源统一。
- 主要文件：新增 runtime 模块，最小改动 `agent.rs`、`mcp_host.rs`、`skills.rs`。
- 依赖：A1、A2。

**B2 Server Control Plane**

- 产出：插件 settings/activation/permissions/health/capabilities API。
- 主要文件：建议把 routes 从 `main.rs` 拆入 `plugins_api.rs`，避免继续放大 `main.rs`。
- 依赖：A1、A2、B1。

**B3 SCM Connector Host**

- 产出：remote matcher、per-remote binding、connector read/mutation handle 和组合事件。
- 不实现 GitHub API；只实现厂商无关 host contract。
- 依赖：A3、B1、B2。

### 批次 C：桌面和首个插件

**C1 Plugin Settings UI**

- 产出：schema-driven settings、权限查看/撤销、scope activation、health 状态。
- 开始修改 UI 前必须遵守仓库 `AGENTS.md`，读取设计系统文档并运行设计检查与类型检查。
- 依赖：B2。

**C2 App View Host**

- 产出：受限 App view 注册、路由、生命周期和消息协议。
- 不允许插件任意导入 renderer 代码或获得 Electron preload 全量接口。
- 依赖：A1、B1。

**C3 Spreadsheet Extraction**

- 产出：官方 Spreadsheet Bundled Plugin，功能与现有 tool/preview 等价。
- 在迁移验收完成前保留兼容 adapter，不一次删除旧实现。
- 依赖：B1、C2。

### 批次 D：GitHub 与高权限能力

**D1 GitHub Connector Integration**

- 使用官方 GitHub 插件已有 Skills/MCP/App 能力。
- 增加 remote matcher 和 account binding adapter，不复制 GitHub 插件的业务逻辑。
- 验证没有 GitHub 插件时 Local Git UI 和 Agent 工具完全可用。
- 依赖：B3、C2。

**D2 Second Connector Conformance Test**

- 使用 GitLab mock connector 或最小测试连接器验证协议。
- 必测：同 workspace 多 remote、同 remote 多 matcher、用户默认选择和解绑。
- 依赖：B3。

**D3 Browser/Computer Packaging**

- 先包装现有 runtime，不在同一任务中重写浏览器或桌面控制实现。
- 验证 approval、sandbox、截图、多模态、下载和 session 恢复。
- 依赖：B1、C2、权限与 Trusted Driver registry。

### 批次 E：Provider 与长尾贡献

**E1 ProviderDriverRegistry**

- 产出：替换多处 `ProviderKind` 构造分支的 registry，但保持设置兼容。
- 普通插件目录不能自动注册 Provider driver。
- 依赖：Trusted Driver 基础类型稳定。

**E2 Preview/Source Contributions**

- 产出：MIME/extension handler registry、优先级、用户默认选择和 fallback。
- 保持 canonical path、安全上限、artifact ownership 在 Host Service。

**E3 Declarative Agent Profiles**

- 产出：插件 profile discovery、来源追踪、冲突规则和 tool allow/deny 上限。
- 插件 profile 不能放宽父 Agent 或平台权限。

## 10. 验收标准

### 10.1 通用插件

- 旧的仅 Skills/MCP 插件无需修改即可发现、安装和启用。
- `apps` 不再只是 `has_apps` 标志，而能得到受限、可停止的运行时。
- 未激活插件不会启动进程、注册工具或向模型注入 prompt。
- 卸载插件不会删除历史 ToolCall/Event 的来源信息。
- Renderer 无法读取 secret 明文。
- 插件工具全部经过 Policy、Approval、Sandbox 和审计。
- 一个插件失败不会导致 Agent Kernel 或其他插件不可用。

### 10.2 Git 与代码托管连接器

- 未安装任何连接器时，status、diff、stage、commit、push、branch 和 worktree 仍可使用。
- GitHub 插件卸载后不影响本地 Git 历史、Diff Review 或 turn undo。
- GitLab/Gitee 等连接器无需修改 Local Git Service 即可接入。
- 同一 workspace 的不同 remote 可以绑定不同连接器和账户。
- 连接器无法直接写受保护的 `.git` 路径。
- “commit + push + create PR/MR”各阶段有独立事件、错误和审批结果。
- 本地 push 成功但远程创建 PR 失败时，UI 和 Agent 能准确呈现部分成功状态。

### 10.3 配置责任

- 平台权限上限不能被 manifest、workspace 或 thread 设置扩大。
- 插件作者声明的 secret 字段不进入普通 JSON settings 或模型上下文。
- 用户能够查看插件为何需要某项权限，并能撤销可选授权。
- 用户禁用插件后，相关工具、Skill、App 和 profile 在同一个 activation snapshot 中消失。
- 官方/第三方/Trusted Driver 的来源和信任等级在 UI 与 API 中可验证，而非插件自报。

## 11. 非目标

本轮设计不要求：

- 建立公网插件市场、计费和自动发布系统。
- 支持任意 Rust/Node 代码在 OpenTopia 主进程或 renderer 内加载。
- 让插件替换 SessionStore、Agent loop、Policy 或 Sandbox。
- 把现有 Local Git 工作台整体迁移到 GitHub 插件。
- 首期开放第三方 Provider、Computer 或 Execution driver 自动加载。
- 在插件底座尚未稳定时同时重写 Browser、Computer 或 Spreadsheet 的领域实现。

## 12. 当前代码对应关系

实现时应以以下现状为迁移起点：

- `ToolRegistry::with_builtins()` 当前直接注册 Browser、Computer 和 Spreadsheet。
- `AgentCore` 当前直接持有 Browser、Computer、Provider、Guardian 和 ToolRegistry。
- `plugins.rs` 当前支持发现、安装、Skills 和 MCP 配置；`apps` 仅被检测并报告尚不支持。
- `PluginDescriptor::is_compatible()` 当前只认可 Skill 或受支持的 MCP server。
- `git_workflow.rs` 已经是结构化、argv-only、基于 `ExecutionEnvironment` 的本地 Git 边界，
  应保留并升级为 Host Service。
- Server 和 Desktop 已经有 status、branch、commit、push、diff/hunk 和 undo 等本地 Git 工作流，
  这些不应依赖 GitHub 插件。
- `ModelProvider` 已是 trait，但 `ProviderKind` 与 Server 构造仍是封闭分支，适合最后迁移为
  Trusted Driver registry，而不是普通插件。

以上边界一旦进入实现，应优先通过协议测试和迁移测试锁定；不要在抽取插件的同时改变现有
用户行为或进行大范围 UI 重写。
