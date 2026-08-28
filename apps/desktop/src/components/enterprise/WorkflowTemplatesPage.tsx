import {
  CheckCircle2,
  Copy,
  FileJson2,
  FlaskConical,
  PauseCircle,
  Play,
  PlayCircle,
  Plus,
  RefreshCw,
  Send,
  ShieldCheck,
  Workflow,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { ApiClient } from "../../api/client";
import type { AgentTemplateVersionView, FlowDraftView } from "../../types";
import { Badge, Button, Panel, Switch, TextField } from "../ui";
import { guidedWorkflowSpec } from "./model";
import {
  useEnterpriseSubpageHeader,
  type EnterprisePageHeaderChange,
} from "./pageHeader";
import { useEnterpriseStore } from "./store";
import { FlowAgentReferencePage } from "./FlowAgentReferencePage";
import { FlowTriggerConfigPage } from "./FlowTriggerConfigPage";
import {
  createManualActivation,
  templateKey,
  type WorkflowAgentSelection,
} from "./flowActivation";
import { WorkflowGraphEditor } from "./WorkflowGraphEditor";

export function WorkflowTemplatesPage({
  client,
  onPageHeaderChange,
  threadId,
}: {
  client: ApiClient;
  onPageHeaderChange?: EnterprisePageHeaderChange;
  threadId: string | null;
}) {
  const { snapshot, store } = useEnterpriseStore(client);
  const publishedTemplates = useMemo(
    () =>
      snapshot.templates.filter((item) => item.template.status === "published"),
    [snapshot.templates],
  );
  const [flowId, setFlowId] = useState("guided-workflow");
  const [name, setName] = useState("Guided workflow");
  const [owner, setOwner] = useState("local_operator");
  const [outcome, setOutcome] = useState("");
  const [agentSelections, setAgentSelections] = useState<
    WorkflowAgentSelection[]
  >([]);
  const [requireApproval, setRequireApproval] = useState(true);
  const [draft, setDraft] = useState<FlowDraftView | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [detailPage, setDetailPage] = useState<{
    kind: "trigger" | "agent";
    nodeId: string;
  } | null>(null);

  const detailAgent = detailPage
    ? (agentSelections.find((item) => item.id === detailPage.nodeId) ?? null)
    : null;
  useEnterpriseSubpageHeader(onPageHeaderChange, creating, {
    title: detailPage
      ? `Flows / ${name} / ${detailPage.kind === "trigger" ? "配置 Trigger" : "Agent 设置"}`
      : "Flows / 创建 Flow",
    backLabel: detailPage ? "返回 Flow 图" : "返回 Flows",
    onBack: () => {
      if (detailPage) setDetailPage(null);
      else setCreating(false);
    },
  });

  useEffect(() => {
    const available = new Set(publishedTemplates.map(templateKey));
    setAgentSelections((current) => {
      const valid = current.filter((item) => available.has(item.templateKey));
      if (valid.length > 0) return valid;
      const first = publishedTemplates[0];
      return first
        ? [
            {
              id: `agent-${crypto.randomUUID()}`,
              templateKey: templateKey(first),
              activation: createManualActivation(),
            },
          ]
        : [];
    });
  }, [publishedTemplates]);

  const selectedAgents = agentSelections
    .map((selection) => {
      const template = publishedTemplates.find(
        (item) => templateKey(item) === selection.templateKey,
      );
      return template ? { selection, template } : null;
    })
    .filter(
      (
        item,
      ): item is {
        selection: WorkflowAgentSelection;
        template: AgentTemplateVersionView;
      } => Boolean(item),
    );
  const passedDryRun = Boolean(
    draft?.trials.some(
      (trial) =>
        trial.draftRevision === draft.draft.revision &&
        trial.status === "passed",
    ),
  );
  const successfulTestRun = Boolean(
    draft?.testRuns.some(
      (run) =>
        run.testDraftRevision === draft.draft.revision &&
        run.definitionContentHash === draft.draft.contentHash &&
        run.status === "succeeded",
    ),
  );
  const activeTestRun = draft?.testRuns.find(
    (run) =>
      run.testDraftRevision === draft.draft.revision && !isTerminal(run.status),
  );

  useEffect(() => {
    if (!activeTestRun) return;
    const timer = window.setInterval(() => {
      void client
        .getFlowRun(activeTestRun.id)
        .then((run) => {
          setDraft((current) =>
            current
              ? {
                  ...current,
                  testRuns: [
                    run,
                    ...current.testRuns.filter((item) => item.id !== run.id),
                  ],
                }
              : current,
          );
        })
        .catch((pollError: unknown) => {
          setError(readableError(pollError));
        });
    }, 1_000);
    return () => window.clearInterval(timer);
  }, [activeTestRun, client]);

  if (!creating) {
    return (
      <div className="enterprise-page enterprise-workflow-templates">
        <Panel
          title="Flows / 工作流"
          actions={
            <Button
              onClick={() => setCreating(true)}
              size="compact"
              variant="primary"
            >
              <Plus aria-hidden="true" size={14} /> New Flow
            </Button>
          }
        >
          <p className="enterprise-page__lede">
            Flow 是可直接运行的事件图：在同一处配置 Agent、节点 Trigger、人工确认、输出与运行状态。
          </p>
          {notice ? (
            <p className="enterprise-page__message is-success" role="status">
              {notice}
            </p>
          ) : null}
          <ol className="enterprise-card-list">
            {snapshot.flows.map((flow) => (
              <li key={flow.flowId}>
                <Workflow aria-hidden="true" size={17} />
                <span>
                  <strong>{flow.name}</strong>
                  <small>
                    {flow.flowId}@{flow.activeRevision.compiledWorkflow.flowVersion} ·{" "}
                    {flow.activeRevision.compiledWorkflow.graph.nodes.length} nodes ·{" "}
                    {flow.activeRevision.trigger.kind.replaceAll("_", " ")}
                  </small>
                </span>
                <div className="enterprise-actions">
                  <Badge variant={flow.status === "active" ? "success" : "neutral"}>
                    {flow.status}
                  </Badge>
                  <Button
                    aria-label={`${flow.status === "active" ? "暂停" : "恢复"} ${flow.name}`}
                    disabled={Boolean(busy)}
                    onClick={() =>
                      void execute(`status:${flow.flowId}`, async () => {
                        if (flow.status === "active") {
                          await client.pauseFlow(flow.flowId, flow.revision);
                        } else {
                          await client.resumeFlow(flow.flowId, flow.revision);
                        }
                        await store.load(true);
                      })
                    }
                    size="compact"
                    variant="quiet"
                  >
                    {flow.status === "active" ? (
                      <PauseCircle aria-hidden="true" size={14} />
                    ) : (
                      <PlayCircle aria-hidden="true" size={14} />
                    )}
                    {flow.status === "active" ? "暂停" : "恢复"}
                  </Button>
                  <Button
                    aria-label={`复制 ${flow.name}`}
                    disabled={Boolean(busy)}
                    onClick={() =>
                      void execute(`copy:${flow.flowId}`, async () => {
                        const suffix = crypto.randomUUID().slice(0, 8);
                        const copyId = `${flow.flowId}-copy-${suffix}`;
                        const copyName = `${flow.name} Copy`;
                        const copied = await client.copyFlow(flow.flowId, {
                          flowId: copyId,
                          name: copyName,
                          owner,
                        });
                        setFlowId(copyId);
                        setName(copyName);
                        setOutcome(copied.draft.spec.description);
                        setDraft(copied);
                        setCreating(true);
                        setNotice("已创建副本草稿；自动 Trigger 已改为人工确认，请复核后再激活。");
                      })
                    }
                    size="compact"
                    variant="quiet"
                  >
                    <Copy aria-hidden="true" size={14} /> 复制
                  </Button>
                </div>
              </li>
            ))}
            {snapshot.flows.length === 0 ? (
              <li className="enterprise-list__empty">尚无已激活 Flow。</li>
            ) : null}
          </ol>
        </Panel>
      </div>
    );
  }

  if (detailPage?.kind === "trigger" && detailAgent) {
    return (
      <FlowTriggerConfigPage
        node={detailAgent}
        onChange={(activation) =>
          setAgentSelections((current) =>
            current.map((item) =>
              item.id === detailAgent.id ? { ...item, activation } : item,
            ),
          )
        }
        selections={agentSelections}
        templates={publishedTemplates}
      />
    );
  }

  if (detailPage?.kind === "agent" && detailAgent) {
    return (
      <FlowAgentReferencePage
        node={detailAgent}
        onChange={(nextTemplateKey) =>
          setAgentSelections((current) =>
            current.map((item) =>
              item.id === detailAgent.id
                ? { ...item, templateKey: nextTemplateKey }
                : item,
            ),
          )
        }
        templates={publishedTemplates}
      />
    );
  }

  async function execute(name: string, action: () => Promise<void>) {
    if (busy) return;
    setBusy(name);
    setError(null);
    setNotice(null);
    try {
      await action();
    } catch (actionError) {
      setError(readableError(actionError));
    } finally {
      setBusy(null);
    }
  }

  function createDraft() {
    if (
      !threadId ||
      selectedAgents.length === 0 ||
      selectedAgents.length !== agentSelections.length ||
      !outcome.trim()
    )
      return;
    void execute("create", async () => {
      const created = await client.createFlowDraft(
        threadId,
        guidedWorkflowSpec({
          flowId: flowId.trim(),
          name: name.trim(),
          owner: owner.trim(),
          outcome: outcome.trim(),
          agents: selectedAgents,
          requireApproval,
        }),
      );
      setDraft(created);
      setNotice("Workflow 草稿已创建；Agent 模板版本已固定。下一步先验证。 ");
    });
  }

  return (
    <div className="enterprise-page enterprise-workflow-templates">
      <Panel title="Flow builder / 工作流图创建">
        <p className="enterprise-page__lede">
          每个节点引用一个独立 Agent，并用 Trigger 表达式订阅外部事件或其他
          Agent 的 Final；通过 Dry Run 与真实 Test Run 后激活不可变 Revision。
        </p>
        {!threadId ? (
          <p className="enterprise-page__message is-warning" role="status">
            请先新建或选择一个 Flow 任务，用于保存草稿与验证记录。
          </p>
        ) : null}
        <div className="enterprise-form-grid">
          <TextField
            label="Workflow ID"
            onChange={(event) => setFlowId(event.target.value)}
            value={flowId}
          />
          <TextField
            label="名称"
            onChange={(event) => setName(event.target.value)}
            value={name}
          />
          <TextField
            label="所有者"
            onChange={(event) => setOwner(event.target.value)}
            value={owner}
          />
          <label className="enterprise-field enterprise-field--wide">
            <span>用自然语言描述希望完成的业务结果</span>
            <textarea
              onChange={(event) => setOutcome(event.target.value)}
              placeholder="例如：读取新客户资料，检查必填字段并生成一份可供销售复核的摘要。"
              rows={4}
              value={outcome}
            />
          </label>
          <WorkflowGraphEditor
            disabled={publishedTemplates.length === 0}
            onChange={setAgentSelections}
            onEditAgent={(nodeId) => setDetailPage({ kind: "agent", nodeId })}
            onEditTrigger={(nodeId) =>
              setDetailPage({ kind: "trigger", nodeId })
            }
            selections={agentSelections}
            templates={publishedTemplates}
          />
          <label className="enterprise-switch enterprise-field--wide">
            <span>
              <strong>执行后人工审查</strong>
              <small>在最后一个 Agent 与 Output 之间增加 Approval 节点。</small>
            </span>
            <Switch checked={requireApproval} onChange={setRequireApproval} />
          </label>
        </div>
        <div className="enterprise-actions">
          <Button
            disabled={
              !threadId ||
              selectedAgents.length === 0 ||
              selectedAgents.length !== agentSelections.length ||
              !outcome.trim() ||
              Boolean(busy)
            }
            onClick={createDraft}
            variant="primary"
          >
            <Plus aria-hidden="true" size={14} />
            {busy === "create" ? "创建中…" : "创建草稿"}
          </Button>
          <Button
            disabled={!draft || Boolean(busy)}
            onClick={() =>
              void execute("validate", async () => {
                setDraft(await client.validateFlowDraft(draft!.draft.id));
                setNotice("静态验证完成，可以进行执行计划 Dry Run。 ");
              })
            }
          >
            <ShieldCheck aria-hidden="true" size={14} /> 验证
          </Button>
          <Button
            disabled={!draft?.draft.lastValidation?.valid || Boolean(busy)}
            onClick={() =>
              void execute("simulate", async () => {
                await client.simulateFlowDraft(draft!.draft.id, {});
                setDraft(
                  (await client.listFlowDrafts(threadId!)).find(
                    (item) => item.draft.id === draft!.draft.id,
                  ) ?? draft,
                );
                setNotice("Dry Run 已通过；下一步执行真实 Test Run。 ");
              })
            }
          >
            <Play aria-hidden="true" size={14} /> Dry Run
          </Button>
          <Button
            disabled={
              !draft?.draft.lastValidation?.valid ||
              !passedDryRun ||
              Boolean(busy) ||
              Boolean(activeTestRun)
            }
            onClick={() =>
              void execute("test-run", async () => {
                const run = await client.startFlowTestRun(
                  draft!.draft.id,
                  {},
                  owner.trim(),
                );
                setDraft((current) =>
                  current
                    ? { ...current, testRuns: [run, ...current.testRuns] }
                    : current,
                );
                setNotice(
                  "真实 Test Run 已启动；Agent、工具和 Connection 会按冻结权限执行。 ",
                );
              })
            }
          >
            <FlaskConical aria-hidden="true" size={14} />
            {activeTestRun ? "测试运行中…" : "Test Run"}
          </Button>
          <Button
            disabled={!draft || Boolean(busy)}
            onClick={() =>
              void execute("refresh-test", async () => {
                setDraft(
                  (await client.listFlowDrafts(threadId!)).find(
                    (item) => item.draft.id === draft!.draft.id,
                  ) ?? draft,
                );
              })
            }
            variant="quiet"
          >
            <RefreshCw aria-hidden="true" size={14} /> 刷新测试状态
          </Button>
          <Button
            disabled={
              !draft?.draft.lastValidation?.valid ||
              !passedDryRun ||
              !successfulTestRun ||
              Boolean(busy)
            }
            onClick={() =>
              void execute("activate", async () => {
                const active = snapshot.flows.find(
                  (flow) => flow.flowId === draft!.draft.spec.flowId,
                );
                await client.activateFlowDraft(draft!.draft.id, {
                  activatedBy: owner.trim(),
                  expectedFlowRevision: active?.revision,
                });
                await store.load(true);
                setNotice("Flow 已激活；Trigger 现在会直接创建 Case，并按入口策略进入待处理或立即运行。 ");
                setCreating(false);
              })
            }
            variant="primary"
          >
            <Send aria-hidden="true" size={14} /> 激活 Flow
          </Button>
        </div>
        {draft ? (
          <WorkflowProgress
            draft={draft}
            passedDryRun={passedDryRun}
            successfulTestRun={successfulTestRun}
          />
        ) : null}
        {error ? (
          <p className="enterprise-page__message is-error" role="alert">
            {error}
          </p>
        ) : null}
        {notice ? (
          <p className="enterprise-page__message is-success" role="status">
            {notice}
          </p>
        ) : null}
        {draft ? (
          <details className="enterprise-advanced">
            <summary>
              <FileJson2 aria-hidden="true" size={15} /> Advanced / 高级 JSON
              检查
            </summary>
            <pre>{JSON.stringify(draft.draft.spec, null, 2)}</pre>
          </details>
        ) : null}
      </Panel>
    </div>
  );
}

function WorkflowProgress({
  draft,
  passedDryRun,
  successfulTestRun,
}: {
  draft: FlowDraftView;
  passedDryRun: boolean;
  successfulTestRun: boolean;
}) {
  const steps = [
    ["Draft", true],
    ["Validated", Boolean(draft.draft.lastValidation?.valid)],
    ["Dry Run", passedDryRun],
    ["Test Run", successfulTestRun],
    ["Activated", draft.draft.status === "published"],
  ] as const;
  return (
    <ol className="enterprise-progress" aria-label="Flow 激活进度">
      {steps.map(([label, done]) => (
        <li className={done ? "is-done" : undefined} key={label}>
          <CheckCircle2 aria-hidden="true" size={15} />
          <span>{label}</span>
        </li>
      ))}
    </ol>
  );
}

function readableError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function isTerminal(status: string): boolean {
  return ["succeeded", "failed", "cancelled"].includes(status);
}
