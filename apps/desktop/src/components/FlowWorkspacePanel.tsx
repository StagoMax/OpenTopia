import { useCallback, useEffect, useMemo, useState } from "react";
import {
  BookOpen,
  Bot,
  Cable,
  CheckCircle2,
  CircleDot,
  Clock3,
  ContactRound,
  Database,
  GitBranch,
  Library,
  MessageSquareText,
  Pause,
  Play,
  Plus,
  RefreshCw,
  RotateCcw,
  Save,
  Send,
  ShieldCheck,
  Square,
  Workflow,
  Wrench,
  XCircle,
} from "lucide-react";
import type { ApiClient } from "../api/client";
import type {
  AppSettings,
  FlowDefinition,
  FlowDraftView,
  FlowNodeKind,
  FlowRun,
  FlowRunStatus,
  FlowSpec,
  FlowTranscriptEntry,
} from "../types";
import { AgentTemplatePanel } from "./AgentTemplatePanel";
import { Badge, Button, IconButton, Panel, Select, TextField } from "./ui";
import "../styles/flow-workspace-panel.css";

export type FlowLibraryConnectorKind = "knowledge" | "database" | "crm";

export type FlowLibraryConnector = {
  id: string;
  kind: FlowLibraryConnectorKind;
  name: string;
  provider?: string;
  description?: string;
  status: "connected" | "syncing" | "attention" | "disabled";
};

export type FlowWorkspacePanelProps = {
  client: ApiClient | null;
  threadId: string | null;
  workspaceRoot: string | null;
  settings: AppSettings | null;
  libraryConnectors?: readonly FlowLibraryConnector[];
  onAddLibraryConnector?(kind: FlowLibraryConnectorKind): void;
  onManageLibraryConnector?(connector: FlowLibraryConnector): void;
};

type PanelTab = "flows" | "agents" | "library";

export function FlowWorkspacePanel({
  client,
  threadId,
  workspaceRoot,
  settings,
  libraryConnectors = [],
  onAddLibraryConnector,
  onManageLibraryConnector,
}: FlowWorkspacePanelProps) {
  const [tab, setTab] = useState<PanelTab>("flows");

  return (
    <div className="flow-workspace-panel">
      <header className="flow-workspace-panel__header">
        <div className="flow-workspace-panel__title">
          <Workflow aria-hidden="true" size={18} />
          <span>
            <strong>Flow 工作台</strong>
            <small>设计、运行与治理</small>
          </span>
        </div>
        <div
          className="flow-workspace-panel__tabs"
          role="tablist"
          aria-label="Flow 工作台"
        >
          <button
            aria-selected={tab === "flows"}
            className={tab === "flows" ? "is-active" : ""}
            onClick={() => setTab("flows")}
            role="tab"
            type="button"
          >
            <GitBranch aria-hidden="true" size={16} />
            Flows
          </button>
          <button
            aria-selected={tab === "agents"}
            className={tab === "agents" ? "is-active" : ""}
            onClick={() => setTab("agents")}
            role="tab"
            type="button"
          >
            <Bot aria-hidden="true" size={16} />
            Agents
          </button>
          <button
            aria-selected={tab === "library"}
            className={tab === "library" ? "is-active" : ""}
            onClick={() => setTab("library")}
            role="tab"
            type="button"
          >
            <Library aria-hidden="true" size={16} />
            Library
          </button>
        </div>
      </header>
      {tab === "flows" ? (
        <FlowReviewPanel
          client={client}
          onOpenLibrary={() => setTab("library")}
          threadId={threadId}
        />
      ) : tab === "agents" ? (
        <AgentTemplatePanel
          client={client}
          threadId={threadId}
          workspaceRoot={workspaceRoot}
          settings={settings}
        />
      ) : (
        <FlowLibraryPanel
          connectors={libraryConnectors}
          onAddConnector={onAddLibraryConnector}
          onManageConnector={onManageLibraryConnector}
        />
      )}
    </div>
  );
}

function FlowReviewPanel({
  client,
  threadId,
  onOpenLibrary,
}: Pick<FlowWorkspacePanelProps, "client" | "threadId"> & {
  onOpenLibrary(): void;
}) {
  const [drafts, setDrafts] = useState<FlowDraftView[]>([]);
  const [library, setLibrary] = useState<FlowDefinition[]>([]);
  const [runs, setRuns] = useState<FlowRun[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [editing, setEditing] = useState(false);
  const [specText, setSpecText] = useState("");
  const [publisher, setPublisher] = useState("");
  const [runDefinitionKey, setRunDefinitionKey] = useState("");
  const [runInputText, setRunInputText] = useState("{}");
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const selected = useMemo(
    () =>
      drafts.find((view) => view.draft.id === selectedId) ?? drafts[0] ?? null,
    [drafts, selectedId],
  );
  const selectedRun = useMemo(
    () => runs.find((run) => run.id === selectedRunId) ?? runs[0] ?? null,
    [runs, selectedRunId],
  );

  const refresh = useCallback(async () => {
    if (!client || !threadId) {
      setDrafts([]);
      setLibrary([]);
      setRuns([]);
      return;
    }
    setError(null);
    try {
      const [nextDrafts, nextLibrary, nextRuns] = await Promise.all([
        client.listFlowDrafts(threadId),
        client.searchFlows(query),
        client.listFlowRuns(threadId),
      ]);
      setDrafts(nextDrafts);
      setLibrary(nextLibrary);
      setRuns(nextRuns);
      setSelectedId((current) =>
        current && nextDrafts.some((view) => view.draft.id === current)
          ? current
          : (nextDrafts[0]?.draft.id ?? null),
      );
      setSelectedRunId((current) =>
        current && nextRuns.some((run) => run.id === current)
          ? current
          : (nextRuns[0]?.id ?? null),
      );
      setRunDefinitionKey((current) =>
        current &&
        nextLibrary.some((flow) => `${flow.flowId}@${flow.version}` === current)
          ? current
          : nextLibrary[0]
            ? `${nextLibrary[0].flowId}@${nextLibrary[0].version}`
            : "",
      );
    } catch (refreshError) {
      setError(readableError(refreshError));
    }
  }, [client, query, threadId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    if (
      !client ||
      !threadId ||
      !runs.some((run) => !isTerminalRunStatus(run.status))
    ) {
      return;
    }
    const timer = window.setInterval(() => {
      void client
        .listFlowRuns(threadId)
        .then((nextRuns) => setRuns(nextRuns))
        .catch((pollError) => setError(readableError(pollError)));
    }, 1500);
    return () => window.clearInterval(timer);
  }, [client, runs, threadId]);

  useEffect(() => {
    if (!editing) {
      setSpecText(selected ? JSON.stringify(selected.draft.spec, null, 2) : "");
    }
  }, [editing, selected]);

  async function runAction(
    name: string,
    action: () => Promise<unknown>,
    message: string,
  ) {
    if (busy) return;
    setBusy(name);
    setError(null);
    setNotice(null);
    try {
      await action();
      setNotice(message);
      await refresh();
    } catch (actionError) {
      setError(readableError(actionError));
    } finally {
      setBusy(null);
    }
  }

  async function createStarter() {
    if (!client || !threadId) return;
    await runAction(
      "create",
      () => client.createFlowDraft(threadId, starterSpec()),
      "已创建可编辑的 Flow 草稿。",
    );
  }

  async function saveSpec() {
    if (!client || !selected) return;
    let spec: FlowSpec;
    try {
      spec = JSON.parse(specText) as FlowSpec;
    } catch (parseError) {
      setError(`Flow spec 不是有效 JSON：${readableError(parseError)}`);
      return;
    }
    await runAction(
      "save",
      () =>
        client.updateFlowDraft(
          selected.draft.id,
          selected.draft.revision,
          spec,
        ),
      "Flow 草稿已保存，等待重新验证。",
    );
    setEditing(false);
  }

  async function startRuntime() {
    if (!client || !threadId || !runDefinitionKey) return;
    let input: unknown;
    try {
      input = JSON.parse(runInputText) as unknown;
    } catch (parseError) {
      setError(`Run input 不是有效 JSON：${readableError(parseError)}`);
      return;
    }
    const separator = runDefinitionKey.lastIndexOf("@");
    const flowId = runDefinitionKey.slice(0, separator);
    const version = Number(runDefinitionKey.slice(separator + 1));
    await runAction(
      "run",
      async () => {
        const run = await client.startFlowRun(threadId, {
          flowId,
          version,
          input,
        });
        setSelectedRunId(run.id);
      },
      "Flow Run 已启动，Trace 会按节点边界持续更新。",
    );
  }

  async function controlRuntime(
    name: string,
    action: () => Promise<FlowRun>,
    message: string,
  ) {
    await runAction(
      name,
      async () => {
        const run = await action();
        setSelectedRunId(run.id);
      },
      message,
    );
  }

  const validation = selected?.draft.lastValidation ?? null;
  const activeRunCount = runs.filter(
    (run) => !isTerminalRunStatus(run.status),
  ).length;
  const approvalCount = runs.filter(
    (run) => run.status === "waiting_approval",
  ).length;
  const successCount = runs.filter((run) => run.status === "succeeded").length;

  function selectPublishedFlow(flow: FlowDefinition) {
    setRunDefinitionKey(`${flow.flowId}@${flow.version}`);
    const relatedDraft = drafts.find(
      (view) => view.draft.spec.flowId === flow.flowId,
    );
    if (relatedDraft) {
      setSelectedId(relatedDraft.draft.id);
      setEditing(false);
    }
  }

  return (
    <div className="flow-review-panel">
      <aside className="flow-review-panel__registry" aria-label="Flow 目录">
        <header className="flow-review-panel__registry-header">
          <span>
            <strong>Flows</strong>
            <small>{drafts.length} 个草稿</small>
          </span>
          <div className="flow-review-panel__actions">
            <IconButton
              aria-label="刷新 Flow 目录"
              disabled={busy !== null}
              onClick={() => void refresh()}
              size="compact"
            >
              <RefreshCw aria-hidden="true" size={14} />
            </IconButton>
            <IconButton
              aria-label="新建 Flow"
              disabled={!client || !threadId || busy !== null}
              onClick={() => void createStarter()}
              size="compact"
              variant="primary"
            >
              <Plus aria-hidden="true" size={14} />
            </IconButton>
          </div>
        </header>
        <TextField
          label="搜索 Flow"
          onChange={(event) => setQuery(event.target.value)}
          placeholder="名称、ID 或负责人"
          type="search"
          value={query}
          wrapperClassName="flow-review-panel__search"
        />
        <nav className="flow-review-panel__drafts" aria-label="Flow 草稿列表">
          <span className="flow-review-panel__nav-label">草稿</span>
          {drafts.map((view) => (
            <button
              className={
                view.draft.id === selected?.draft.id ? "is-active" : ""
              }
              key={view.draft.id}
              onClick={() => {
                setSelectedId(view.draft.id);
                setEditing(false);
              }}
              type="button"
            >
              <GitBranch aria-hidden="true" size={16} />
              <span>
                <strong>{view.draft.spec.name}</strong>
                <small>
                  {view.draft.spec.flowId} · r{view.draft.revision}
                </small>
              </span>
              <StatusBadge status={view.draft.status} />
            </button>
          ))}
          {drafts.length === 0 ? (
            <p className="flow-review-panel__empty">
              在对话中描述工作流程，Agent 会调用 flow_create
              生成草稿；也可先创建空白草稿。
            </p>
          ) : null}
          {library.length > 0 ? (
            <>
              <span className="flow-review-panel__nav-label">已发布</span>
              {library.map((flow) => {
                const key = `${flow.flowId}@${flow.version}`;
                return (
                  <button
                    className={key === runDefinitionKey ? "is-active" : ""}
                    key={key}
                    onClick={() => selectPublishedFlow(flow)}
                    type="button"
                  >
                    <Library aria-hidden="true" size={16} />
                    <span>
                      <strong>{flow.name}</strong>
                      <small>
                        {flow.flowId} · v{flow.version}
                      </small>
                    </span>
                    <Badge variant="success">live</Badge>
                  </button>
                );
              })}
            </>
          ) : null}
        </nav>
        <button
          className="flow-review-panel__library-link"
          onClick={onOpenLibrary}
          type="button"
        >
          <BookOpen aria-hidden="true" size={16} />
          <span>
            <strong>Library & Connectors</strong>
            <small>RAG、数据库与 CRM</small>
          </span>
        </button>
      </aside>

      <main className="flow-review-panel__main">
        <header className="flow-review-panel__main-header">
          <span>
            <small>Operations</small>
            <strong>{selected?.draft.spec.name ?? "Flow Runtime"}</strong>
          </span>
          {selected ? <StatusBadge status={selected.draft.status} /> : null}
        </header>

        <section className="flow-review-panel__metrics" aria-label="运行概览">
          <article>
            <span>全部运行</span>
            <strong>{runs.length}</strong>
            <small>{activeRunCount} 个进行中</small>
          </article>
          <article>
            <span>等待审批</span>
            <strong>{approvalCount}</strong>
            <small>{approvalCount ? "需要人工处理" : "当前无阻塞"}</small>
          </article>
          <article>
            <span>成功完成</span>
            <strong>{successCount}</strong>
            <small>
              {runs.length
                ? `${Math.round((successCount / runs.length) * 100)}% 通过率`
                : "尚无数据"}
            </small>
          </article>
        </section>

        {error ? (
          <p className="flow-review-panel__message is-error" role="alert">
            {error}
          </p>
        ) : null}
        {notice ? (
          <p className="flow-review-panel__message is-success" role="status">
            {notice}
          </p>
        ) : null}

        <FlowRuntimePanel
          busy={busy}
          client={client}
          library={library}
          onControl={(name, action, message) =>
            void controlRuntime(name, action, message)
          }
          onDefinitionChange={setRunDefinitionKey}
          onInputChange={setRunInputText}
          onRun={() => void startRuntime()}
          onSelectRun={setSelectedRunId}
          runDefinitionKey={runDefinitionKey}
          runInputText={runInputText}
          runs={runs}
          selectedRun={selectedRun}
        />
      </main>

      <aside
        className="flow-review-panel__inspector"
        aria-label="Flow 配置与治理"
      >
        {selected ? (
          <Panel
            title={selected.draft.spec.name}
            actions={<StatusBadge status={selected.draft.status} />}
          >
            <dl className="flow-review-panel__facts">
              <div>
                <dt>Owner</dt>
                <dd>{selected.draft.spec.owner}</dd>
              </div>
              <div>
                <dt>Source</dt>
                <dd>{sourceLabel(selected.draft.spec)}</dd>
              </div>
              <div>
                <dt>Risk</dt>
                <dd>
                  <Badge
                    variant={riskBadgeVariant(selected.draft.spec.riskClass)}
                  >
                    {selected.draft.spec.riskClass}
                  </Badge>
                </dd>
              </div>
              <div>
                <dt>Hash</dt>
                <dd>
                  <code>{selected.draft.contentHash}</code>
                </dd>
              </div>
              <div>
                <dt>Tools</dt>
                <dd>
                  {selected.draft.effectiveCapabilities.allowAllTools
                    ? "all visible tools"
                    : selected.draft.effectiveCapabilities.tools.join(", ") ||
                      "none"}
                </dd>
              </div>
              <div>
                <dt>Skills</dt>
                <dd>
                  {selected.draft.effectiveCapabilities.allowAllSkills
                    ? "all visible Skills"
                    : selected.draft.effectiveCapabilities.skills.join(", ") ||
                      "none"}
                </dd>
              </div>
            </dl>

            <div className="flow-review-panel__section-heading">
              <span>
                <GitBranch aria-hidden="true" size={15} /> Graph
              </span>
              <Button
                onClick={() => setEditing((value) => !value)}
                size="compact"
                variant="quiet"
              >
                {editing ? "取消编辑" : "编辑 Spec"}
              </Button>
            </div>

            {editing ? (
              <label className="flow-review-panel__editor">
                <span>Flow spec JSON</span>
                <textarea
                  aria-label="Flow spec JSON"
                  onChange={(event) => setSpecText(event.target.value)}
                  spellCheck={false}
                  value={specText}
                />
                <Button
                  disabled={busy !== null}
                  onClick={() => void saveSpec()}
                  variant="primary"
                >
                  <Save aria-hidden="true" size={14} /> 保存 revision
                </Button>
              </label>
            ) : (
              <GraphInspector spec={selected.draft.spec} />
            )}

            <div className="flow-review-panel__section-heading">
              <span>
                <ShieldCheck aria-hidden="true" size={15} /> Validation
              </span>
              <Badge
                variant={
                  validation?.valid
                    ? "success"
                    : validation
                      ? "danger"
                      : "neutral"
                }
              >
                {validation
                  ? validation.valid
                    ? "passed"
                    : `${validation.issues.length} issues`
                  : "not run"}
              </Badge>
            </div>
            {validation?.issues.length ? (
              <ul className="flow-review-panel__issues">
                {validation.issues.map((issue, index) => (
                  <li key={`${issue.code}-${index}`}>
                    <strong>{issue.code}</strong>
                    <span>{issue.message}</span>
                    <small>{issue.remediation}</small>
                  </li>
                ))}
              </ul>
            ) : (
              <p className="flow-review-panel__empty">尚无校验问题。</p>
            )}

            <div className="flow-review-panel__action-grid">
              <Button
                disabled={busy !== null}
                onClick={() =>
                  void runAction(
                    "validate",
                    () => client!.validateFlowDraft(selected.draft.id),
                    "静态验证已完成。",
                  )
                }
              >
                <CheckCircle2 aria-hidden="true" size={14} /> 验证
              </Button>
              <Button
                disabled={busy !== null}
                onClick={() =>
                  void runAction(
                    "simulate",
                    () => client!.simulateFlowDraft(selected.draft.id),
                    "Harness 模拟已记录。",
                  )
                }
              >
                <Play aria-hidden="true" size={14} /> 模拟
              </Button>
            </div>

            {selected.trials[0] ? (
              <details className="flow-review-panel__trial">
                <summary>
                  <strong>最近模拟：{selected.trials[0].status}</strong>
                  <span>
                    {selected.trials[0].steps.length} 个 Harness 节点 · revision{" "}
                    {selected.trials[0].draftRevision}
                  </span>
                </summary>
                <ol>
                  {selected.trials[0].steps.map((step) => (
                    <li key={`${step.order}-${step.nodeId}`}>
                      <code>
                        {step.order + 1}. {step.nodeId} → {step.harnessTarget}
                        {step.boundedBy ? ` · max ${step.boundedBy}` : ""}
                      </code>
                    </li>
                  ))}
                </ol>
              </details>
            ) : null}

            <TextField
              hint="高风险 Flow 必须由 owner 之外的人发布。"
              label="发布审批人"
              onChange={(event) => setPublisher(event.target.value)}
              placeholder="姓名或企业身份 ID"
              value={publisher}
            />
            <Button
              disabled={
                !client || busy !== null || publisher.trim().length === 0
              }
              onClick={() =>
                void runAction(
                  "publish",
                  () =>
                    client!.publishFlowDraft(
                      selected.draft.id,
                      publisher.trim(),
                    ),
                  "已发布不可变 Flow 版本。",
                )
              }
              variant="primary"
            >
              <Send aria-hidden="true" size={14} /> 发布 Flow
            </Button>
          </Panel>
        ) : (
          <div className="flow-review-panel__inspector-empty">
            <GitBranch aria-hidden="true" size={20} />
            <strong>选择一个 Flow</strong>
            <span>查看图结构、权限、验证结果与发布控制。</span>
          </div>
        )}
        <Panel
          className="flow-review-panel__connections"
          title="Business Context"
          actions={<Badge>接口预留</Badge>}
        >
          <p>
            将 Flow 连接到知识库、业务数据库与 CRM。连接器配置会在 Library
            中集中管理。
          </p>
          <Button onClick={onOpenLibrary} size="compact" variant="quiet">
            <Cable aria-hidden="true" size={14} /> 打开 Library
          </Button>
        </Panel>
      </aside>
    </div>
  );
}

type FlowLibraryPanelProps = {
  connectors: readonly FlowLibraryConnector[];
  onAddConnector?: (kind: FlowLibraryConnectorKind) => void;
  onManageConnector?: (connector: FlowLibraryConnector) => void;
};

const FLOW_LIBRARY_KINDS: ReadonlyArray<{
  kind: FlowLibraryConnectorKind;
  title: string;
  description: string;
}> = [
  {
    kind: "knowledge",
    title: "知识库 / RAG",
    description: "接入文档、网页与向量索引，为 Flow 提供可追溯的检索上下文。",
  },
  {
    kind: "database",
    title: "业务数据库",
    description: "连接 SQL、数据仓库或内部数据服务，并按身份授予读写范围。",
  },
  {
    kind: "crm",
    title: "CRM",
    description: "连接客户、销售线索与机会数据，供业务 Flow 查询或更新。",
  },
];

function FlowLibraryPanel({
  connectors,
  onAddConnector,
  onManageConnector,
}: FlowLibraryPanelProps) {
  return (
    <div className="flow-library-panel">
      <header className="flow-library-panel__hero">
        <span className="flow-library-panel__hero-icon">
          <Library aria-hidden="true" size={20} />
        </span>
        <span>
          <small>Business Context</small>
          <h2>Library & Connectors</h2>
          <p>
            为 Flow 和 Agent
            提供统一的企业知识与系统连接。此处已预留稳定的前端接口，后续可直接接入
            RAG、数据库和 CRM 管理流程。
          </p>
        </span>
        <Badge variant="info">API ready</Badge>
      </header>

      <section
        className="flow-library-panel__catalog"
        aria-labelledby="connector-catalog-title"
      >
        <header>
          <span>
            <h3 id="connector-catalog-title">添加连接</h3>
            <p>选择上下文来源；连接、鉴权与同步逻辑由后续实现注入。</p>
          </span>
        </header>
        <div className="flow-library-panel__cards">
          {FLOW_LIBRARY_KINDS.map((item) => (
            <article key={item.kind}>
              <span className="flow-library-panel__card-icon">
                {connectorKindIcon(item.kind)}
              </span>
              <span>
                <strong>{item.title}</strong>
                <small>{item.description}</small>
              </span>
              <Button
                disabled={!onAddConnector}
                onClick={() => onAddConnector?.(item.kind)}
                size="compact"
                title={
                  onAddConnector
                    ? `添加${item.title}`
                    : "连接器接口已预留，等待后端接入"
                }
                variant="quiet"
              >
                <Plus aria-hidden="true" size={14} /> 添加
              </Button>
            </article>
          ))}
        </div>
      </section>

      <section
        className="flow-library-panel__connected"
        aria-labelledby="connected-sources-title"
      >
        <header>
          <span>
            <h3 id="connected-sources-title">已连接来源</h3>
            <p>连接状态、同步与权限会集中显示在这里。</p>
          </span>
          <Badge>{connectors.length}</Badge>
        </header>
        {connectors.length ? (
          <div className="flow-library-panel__source-list">
            {connectors.map((connector) => (
              <article key={connector.id}>
                <span className="flow-library-panel__source-icon">
                  {connectorKindIcon(connector.kind)}
                </span>
                <span>
                  <strong>{connector.name}</strong>
                  <small>
                    {[connector.provider, connector.description]
                      .filter(Boolean)
                      .join(" · ")}
                  </small>
                </span>
                <ConnectorStatusBadge status={connector.status} />
                <Button
                  disabled={!onManageConnector}
                  onClick={() => onManageConnector?.(connector)}
                  size="compact"
                  variant="quiet"
                >
                  管理
                </Button>
              </article>
            ))}
          </div>
        ) : (
          <div className="flow-library-panel__empty">
            <Cable aria-hidden="true" size={20} />
            <strong>尚未连接外部来源</strong>
            <span>
              接入后，Flow 可以在明确的身份和权限边界内读取业务上下文。
            </span>
          </div>
        )}
      </section>
    </div>
  );
}

function connectorKindIcon(kind: FlowLibraryConnectorKind) {
  if (kind === "knowledge") return <BookOpen aria-hidden="true" size={18} />;
  if (kind === "database") return <Database aria-hidden="true" size={18} />;
  return <ContactRound aria-hidden="true" size={18} />;
}

function ConnectorStatusBadge({
  status,
}: {
  status: FlowLibraryConnector["status"];
}) {
  const variant =
    status === "connected"
      ? "success"
      : status === "syncing"
        ? "info"
        : status === "attention"
          ? "warning"
          : "neutral";
  const label = {
    connected: "已连接",
    syncing: "同步中",
    attention: "需处理",
    disabled: "已停用",
  }[status];
  return <Badge variant={variant}>{label}</Badge>;
}

function GraphInspector({ spec }: { spec: FlowSpec }) {
  return (
    <ol className="flow-review-panel__graph">
      {spec.graph.nodes.map((node) => {
        const outgoing = spec.graph.edges.filter(
          (edge) => edge.from === node.id,
        );
        return (
          <li key={node.id}>
            <span className="flow-review-panel__node-icon">
              {nodeIcon(node.kind)}
            </span>
            <div>
              <strong>{node.label}</strong>
              <small>
                {node.kind} · {node.id}
              </small>
              {outgoing.map((edge, index) => (
                <code key={`${edge.to}-${index}`}>
                  → {edge.to}
                  {edge.condition ? ` if ${edge.condition}` : ""}
                  {edge.loopPolicy
                    ? ` · max ${edge.loopPolicy.maxIterations}`
                    : ""}
                </code>
              ))}
            </div>
          </li>
        );
      })}
    </ol>
  );
}

type FlowRuntimePanelProps = {
  busy: string | null;
  client: ApiClient | null;
  library: FlowDefinition[];
  runs: FlowRun[];
  selectedRun: FlowRun | null;
  runDefinitionKey: string;
  runInputText: string;
  onDefinitionChange(value: string): void;
  onInputChange(value: string): void;
  onRun(): void;
  onSelectRun(runId: string): void;
  onControl(
    name: string,
    action: () => Promise<FlowRun>,
    message: string,
  ): void;
};

function FlowRuntimePanel({
  busy,
  client,
  library,
  runs,
  selectedRun,
  runDefinitionKey,
  runInputText,
  onDefinitionChange,
  onInputChange,
  onRun,
  onSelectRun,
  onControl,
}: FlowRuntimePanelProps) {
  const canPause = selectedRun
    ? selectedRun.status === "queued" || selectedRun.status === "running"
    : false;
  const canResume = selectedRun?.status === "paused";
  const hasInterruptedNode = selectedRun?.nodeRuns.some(
    (nodeRun) => nodeRun.status === "running",
  );
  const waitingApproval = selectedRun?.status === "waiting_approval";
  const canCancel = selectedRun
    ? !isTerminalRunStatus(selectedRun.status) &&
      selectedRun.status !== "cancel_requested"
    : false;

  return (
    <Panel
      className="flow-runtime-panel"
      title="Runtime & Trace"
      actions={
        selectedRun ? <RunStatusBadge status={selectedRun.status} /> : undefined
      }
    >
      <div className="flow-runtime-panel__composer">
        <Select
          disabled={busy !== null || library.length === 0}
          label="选择已发布 Flow"
          onChange={onDefinitionChange}
          options={library.map((flow) => ({
            value: `${flow.flowId}@${flow.version}`,
            label: `${flow.name} · v${flow.version}`,
          }))}
          value={runDefinitionKey}
        />
        <label className="flow-review-panel__editor">
          <span>Run input JSON</span>
          <textarea
            aria-label="Flow Run input JSON"
            onChange={(event) => onInputChange(event.target.value)}
            spellCheck={false}
            value={runInputText}
          />
        </label>
        <Button
          disabled={
            !client ||
            busy !== null ||
            library.length === 0 ||
            !runDefinitionKey
          }
          onClick={onRun}
          variant="primary"
        >
          <Play aria-hidden="true" size={14} />
          {busy === "run" ? "启动中…" : "运行 Flow"}
        </Button>
      </div>

      <div className="flow-runtime-panel__runs" aria-label="Flow Run 列表">
        {runs.map((run) => (
          <button
            className={run.id === selectedRun?.id ? "is-active" : ""}
            key={run.id}
            onClick={() => onSelectRun(run.id)}
            type="button"
          >
            <Clock3 aria-hidden="true" size={16} />
            <span>
              <strong>
                {run.flowId}@{run.flowVersion}
              </strong>
              <small>
                {run.nodeExecutions}/{run.budget.maxNodeExecutions} nodes ·{" "}
                {run.toolCalls}/{run.budget.maxToolCalls} tools
              </small>
            </span>
            <RunStatusBadge status={run.status} />
          </button>
        ))}
        {runs.length === 0 ? (
          <p className="flow-review-panel__empty">
            发布 Flow 后可从这里启动。每个节点的输入、输出、重试和错误都会保存在
            Trace 中。
          </p>
        ) : null}
      </div>

      {selectedRun ? (
        <div className="flow-runtime-panel__detail" aria-live="polite">
          <dl className="flow-review-panel__facts">
            <div>
              <dt>Run</dt>
              <dd>
                <code>{selectedRun.id}</code>
              </dd>
            </div>
            <div>
              <dt>Ready</dt>
              <dd>{selectedRun.readyNodes.join(", ") || "none"}</dd>
            </div>
            <div>
              <dt>Loops</dt>
              <dd>{formatLoopCounts(selectedRun.loopCounts)}</dd>
            </div>
            <div>
              <dt>Updated</dt>
              <dd>{new Date(selectedRun.updatedAt).toLocaleString()}</dd>
            </div>
          </dl>

          <div
            className="flow-runtime-panel__controls"
            aria-label="Flow Run 控制"
          >
            {canPause ? (
              <Button
                disabled={!client || busy !== null}
                onClick={() =>
                  onControl(
                    "pause-run",
                    () => client!.pauseFlowRun(selectedRun.id),
                    "已请求在下一个节点边界暂停。",
                  )
                }
                size="compact"
              >
                <Pause aria-hidden="true" size={14} /> 暂停
              </Button>
            ) : null}
            {canResume ? (
              <Button
                disabled={!client || busy !== null}
                onClick={() => {
                  if (
                    hasInterruptedNode &&
                    !window.confirm(
                      "上次进程在节点执行中停止。请先核对外部系统是否已产生副作用；确认后会创建一次新的节点尝试。",
                    )
                  ) {
                    return;
                  }
                  onControl(
                    "resume-run",
                    () =>
                      client!.resumeFlowRun(selectedRun.id, {
                        retryInterruptedNode: hasInterruptedNode,
                      }),
                    hasInterruptedNode
                      ? "中断节点已按人工确认创建新尝试。"
                      : "Flow 已从持久化检查点恢复。",
                  );
                }}
                size="compact"
              >
                <RotateCcw aria-hidden="true" size={14} />
                {hasInterruptedNode ? "检查后重试" : "恢复"}
              </Button>
            ) : null}
            {waitingApproval ? (
              <>
                <Button
                  disabled={!client || busy !== null}
                  onClick={() =>
                    onControl(
                      "approve-run",
                      () =>
                        client!.resumeFlowRun(selectedRun.id, {
                          approved: true,
                        }),
                      "审批通过，Flow 已继续运行。",
                    )
                  }
                  size="compact"
                  variant="primary"
                >
                  <CheckCircle2 aria-hidden="true" size={14} /> 通过
                </Button>
                <Button
                  disabled={!client || busy !== null}
                  onClick={() =>
                    onControl(
                      "reject-run",
                      () =>
                        client!.resumeFlowRun(selectedRun.id, {
                          approved: false,
                          note: "Rejected from Flow review panel",
                        }),
                      "审批已拒绝，Flow Run 已取消。",
                    )
                  }
                  size="compact"
                  variant="danger"
                >
                  <XCircle aria-hidden="true" size={14} /> 拒绝
                </Button>
              </>
            ) : null}
            {canCancel ? (
              <Button
                disabled={!client || busy !== null}
                onClick={() =>
                  onControl(
                    "cancel-run",
                    () => client!.cancelFlowRun(selectedRun.id),
                    "已请求在下一个节点边界取消。",
                  )
                }
                size="compact"
                variant="danger"
              >
                <Square aria-hidden="true" size={14} /> 取消
              </Button>
            ) : null}
          </div>

          {selectedRun.error ? (
            <p className="flow-review-panel__message is-error" role="alert">
              {selectedRun.error}
            </p>
          ) : null}

          <div className="flow-review-panel__section-heading">
            <span>
              <GitBranch aria-hidden="true" size={15} /> Node Trace
            </span>
            <Badge variant="neutral">
              {selectedRun.nodeRuns.length} attempts
            </Badge>
          </div>
          <ol className="flow-runtime-panel__trace">
            {selectedRun.nodeRuns.map((nodeRun) => (
              <li key={nodeRun.id}>
                <span className="flow-runtime-panel__trace-index">
                  {nodeRun.attempt}
                </span>
                <div>
                  <span className="flow-runtime-panel__trace-title">
                    <strong>{nodeRun.nodeId}</strong>
                    <RunStatusBadge status={nodeRun.status} />
                  </span>
                  <small>
                    {nodeRun.toolCalls} tool calls ·{" "}
                    {formatDuration(nodeRun.startedAt, nodeRun.completedAt)}
                  </small>
                  {nodeRun.error ? <code>{nodeRun.error}</code> : null}
                  {nodeRun.transcript.length > 0 ? (
                    <details
                      className="flow-runtime-panel__transcript"
                      open={nodeRun.transcript.length <= 6}
                    >
                      <summary>
                        <span>
                          <MessageSquareText aria-hidden="true" size={14} />
                          对话与工具过程
                        </span>
                        <Badge variant="neutral">
                          {nodeRun.transcript.length} 条
                        </Badge>
                      </summary>
                      <ol aria-label={`${nodeRun.nodeId} 节点对话过程`}>
                        {nodeRun.transcript.map((entry) => (
                          <TranscriptEntry entry={entry} key={entry.id} />
                        ))}
                      </ol>
                    </details>
                  ) : null}
                  {nodeRun.output !== null ? (
                    <details>
                      <summary>查看输出</summary>
                      <pre>{JSON.stringify(nodeRun.output, null, 2)}</pre>
                    </details>
                  ) : null}
                </div>
              </li>
            ))}
          </ol>
        </div>
      ) : null}
    </Panel>
  );
}

function TranscriptEntry({ entry }: { entry: FlowTranscriptEntry }) {
  return (
    <li
      className={entry.isError ? "is-error" : undefined}
      data-kind={entry.kind}
    >
      <span className="flow-runtime-panel__transcript-icon">
        {transcriptIcon(entry.kind)}
      </span>
      <div>
        <span className="flow-runtime-panel__transcript-title">
          <strong>{transcriptKindLabel(entry.kind)}</strong>
          <small>{entry.title}</small>
        </span>
        <pre>{formatTranscriptContent(entry.content)}</pre>
      </div>
    </li>
  );
}

function transcriptIcon(kind: FlowTranscriptEntry["kind"]) {
  if (kind === "tool_call" || kind === "tool_result")
    return <Wrench aria-hidden="true" size={14} />;
  if (kind === "approval") return <ShieldCheck aria-hidden="true" size={14} />;
  if (kind === "error") return <XCircle aria-hidden="true" size={14} />;
  if (kind === "output") return <CircleDot aria-hidden="true" size={14} />;
  return <Send aria-hidden="true" size={14} />;
}

function transcriptKindLabel(kind: FlowTranscriptEntry["kind"]) {
  if (kind === "input") return "输入";
  if (kind === "tool_call") return "工具调用";
  if (kind === "tool_result") return "工具结果";
  if (kind === "output") return "节点输出";
  if (kind === "approval") return "人工审批";
  return "错误";
}

function formatTranscriptContent(content: unknown) {
  if (typeof content === "string") return content;
  return JSON.stringify(content, null, 2) ?? String(content);
}

function nodeIcon(kind: FlowNodeKind) {
  if (kind === "agent") return <Bot aria-hidden="true" size={15} />;
  if (kind === "tool" || kind === "skill")
    return <Wrench aria-hidden="true" size={15} />;
  if (kind === "validator" || kind === "approval")
    return <ShieldCheck aria-hidden="true" size={15} />;
  if (kind === "output") return <CheckCircle2 aria-hidden="true" size={15} />;
  return <CircleDot aria-hidden="true" size={15} />;
}

function StatusBadge({ status }: { status: FlowDraftView["draft"]["status"] }) {
  const variant =
    status === "published"
      ? "success"
      : status === "ready_to_publish"
        ? "info"
        : "neutral";
  return <Badge variant={variant}>{status.replaceAll("_", " ")}</Badge>;
}

function riskBadgeVariant(
  risk: FlowSpec["riskClass"],
): "neutral" | "warning" | "danger" {
  if (risk === "critical") return "danger";
  if (risk === "high") return "warning";
  return "neutral";
}

function RunStatusBadge({ status }: { status: FlowRunStatus }) {
  const variant =
    status === "succeeded"
      ? "success"
      : status === "failed" || status === "cancelled"
        ? "danger"
        : status === "waiting_approval" || status === "paused"
          ? "warning"
          : status === "running"
            ? "info"
            : "neutral";
  return <Badge variant={variant}>{status.replaceAll("_", " ")}</Badge>;
}

function isTerminalRunStatus(status: FlowRunStatus) {
  return (
    status === "succeeded" || status === "failed" || status === "cancelled"
  );
}

function formatLoopCounts(loopCounts: Record<string, number>) {
  const entries = Object.entries(loopCounts);
  return entries.length
    ? entries.map(([edge, count]) => `edge ${edge}: ${count}`).join(", ")
    : "none";
}

function formatDuration(startedAt: string, completedAt: string | null) {
  const elapsed =
    new Date(completedAt ?? Date.now()).getTime() -
    new Date(startedAt).getTime();
  if (!Number.isFinite(elapsed) || elapsed < 0) return "unknown duration";
  if (elapsed < 1000) return `${elapsed} ms`;
  return `${(elapsed / 1000).toFixed(1)} s`;
}

function sourceLabel(spec: FlowSpec) {
  return spec.source.kind === "run_trace"
    ? `Run/Trace ${spec.source.runId}`
    : "Natural language";
}

function starterSpec(): FlowSpec {
  return {
    flowId: `new-flow-${Date.now()}`,
    name: "New Flow",
    description:
      "Describe the reusable outcome and review the generated graph.",
    owner: "unassigned",
    categories: [],
    source: {
      kind: "natural_language",
      description: "Created from the Flow review surface.",
    },
    inputSchema: { type: "object" },
    outputSchema: { type: "object" },
    graph: {
      schemaVersion: 1,
      entryNodeId: "output",
      nodes: [
        {
          id: "output",
          label: "Return result",
          kind: "output",
          config: {},
          inputSchema: { type: "object" },
          outputSchema: { type: "object" },
        },
      ],
      edges: [],
    },
    requestedCapabilities: {
      allowAllTools: false,
      tools: [],
      allowAllSkills: false,
      skills: [],
      allowAllPlugins: false,
      plugins: [],
      allowAllMcpServers: false,
      mcpServers: [],
      allowAllWorkspaceRoots: false,
      workspaceRoots: [],
    },
    budget: {
      maxNodeExecutions: 100,
      maxToolCalls: 60,
      maxDurationSeconds: 3600,
      maxLoopIterations: 10,
    },
    riskClass: "low",
    pendingDecisions: [],
  };
}

function readableError(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}
