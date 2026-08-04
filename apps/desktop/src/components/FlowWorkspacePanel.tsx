import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Bot,
  CheckCircle2,
  CircleDot,
  GitBranch,
  Library,
  Play,
  Plus,
  RefreshCw,
  Save,
  Search,
  Send,
  ShieldCheck,
  Wrench,
} from "lucide-react";
import type { ApiClient } from "../api/client";
import type {
  AppSettings,
  FlowDefinition,
  FlowDraftView,
  FlowNodeKind,
  FlowSpec,
} from "../types";
import { AgentTemplatePanel } from "./AgentTemplatePanel";
import { Badge, Button, Panel, TextField } from "./ui";
import "../styles/flow-workspace-panel.css";

type FlowWorkspacePanelProps = {
  client: ApiClient | null;
  threadId: string | null;
  workspaceRoot: string | null;
  settings: AppSettings | null;
};

type PanelTab = "flows" | "agents";

export function FlowWorkspacePanel({
  client,
  threadId,
  workspaceRoot,
  settings,
}: FlowWorkspacePanelProps) {
  const [tab, setTab] = useState<PanelTab>("flows");

  return (
    <div className="flow-workspace-panel">
      <div
        className="flow-workspace-panel__tabs"
        role="tablist"
        aria-label="Flow workspace"
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
      </div>
      {tab === "flows" ? (
        <FlowReviewPanel client={client} threadId={threadId} />
      ) : (
        <AgentTemplatePanel
          client={client}
          threadId={threadId}
          workspaceRoot={workspaceRoot}
          settings={settings}
        />
      )}
    </div>
  );
}

function FlowReviewPanel({
  client,
  threadId,
}: Pick<FlowWorkspacePanelProps, "client" | "threadId">) {
  const [drafts, setDrafts] = useState<FlowDraftView[]>([]);
  const [library, setLibrary] = useState<FlowDefinition[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [editing, setEditing] = useState(false);
  const [specText, setSpecText] = useState("");
  const [publisher, setPublisher] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const selected = useMemo(
    () =>
      drafts.find((view) => view.draft.id === selectedId) ?? drafts[0] ?? null,
    [drafts, selectedId],
  );

  const refresh = useCallback(async () => {
    if (!client || !threadId) {
      setDrafts([]);
      setLibrary([]);
      return;
    }
    setError(null);
    try {
      const [nextDrafts, nextLibrary] = await Promise.all([
        client.listFlowDrafts(threadId),
        client.searchFlows(query),
      ]);
      setDrafts(nextDrafts);
      setLibrary(nextLibrary);
      setSelectedId((current) =>
        current && nextDrafts.some((view) => view.draft.id === current)
          ? current
          : (nextDrafts[0]?.draft.id ?? null),
      );
    } catch (refreshError) {
      setError(readableError(refreshError));
    }
  }, [client, query, threadId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

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

  const validation = selected?.draft.lastValidation ?? null;

  return (
    <div className="flow-review-panel">
      <Panel
        title="Flow Registry"
        actions={
          <div className="flow-review-panel__actions">
            <Button
              aria-label="刷新 Flow Registry"
              disabled={busy !== null}
              onClick={() => void refresh()}
              size="compact"
              variant="quiet"
            >
              <RefreshCw aria-hidden="true" size={14} />
            </Button>
            <Button
              disabled={!client || !threadId || busy !== null}
              onClick={() => void createStarter()}
              size="compact"
              variant="primary"
            >
              <Plus aria-hidden="true" size={14} /> 新建
            </Button>
          </div>
        }
      >
        <TextField
          label="搜索已发布 Flow"
          onChange={(event) => setQuery(event.target.value)}
          placeholder="名称、ID 或负责人"
          type="search"
          value={query}
        />
        <div className="flow-review-panel__drafts" aria-label="Flow 草稿列表">
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
              在对话中描述工作流程，Agent 会调用 flow.create
              生成草稿；也可先创建空白草稿。
            </p>
          ) : null}
        </div>
        {library.length > 0 ? (
          <details className="flow-review-panel__library">
            <summary>
              <Library aria-hidden="true" size={14} /> 已发布（{library.length}
              ）
            </summary>
            <ul>
              {library.map((flow) => (
                <li key={`${flow.flowId}@${flow.version}`}>
                  <span>{flow.name}</span>
                  <code>
                    {flow.flowId}@{flow.version}
                  </code>
                </li>
              ))}
            </ul>
          </details>
        ) : null}
      </Panel>

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
              <dd>{selected.draft.spec.riskClass}</dd>
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
            disabled={busy !== null || publisher.trim().length === 0}
            onClick={() =>
              void runAction(
                "publish",
                () =>
                  client!.publishFlowDraft(selected.draft.id, publisher.trim()),
                "已发布不可变 Flow 版本。",
              )
            }
            variant="primary"
          >
            <Send aria-hidden="true" size={14} /> 发布 Flow
          </Button>
        </Panel>
      ) : null}
    </div>
  );
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
