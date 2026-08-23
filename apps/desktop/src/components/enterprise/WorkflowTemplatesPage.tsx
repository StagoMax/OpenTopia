import {
  CheckCircle2,
  FileJson2,
  FlaskConical,
  Play,
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
import {
  WorkflowAgentSequenceEditor,
  type WorkflowAgentSelection,
} from "./WorkflowAgentSequenceEditor";

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

  useEnterpriseSubpageHeader(onPageHeaderChange, creating, {
    title: "Workflow Templates / 创建工作流模板",
    backLabel: "返回 Workflow Templates",
    onBack: () => setCreating(false),
  });

  useEffect(() => {
    const available = new Set(publishedTemplates.map(keyOf));
    setAgentSelections((current) => {
      const valid = current.filter((item) => available.has(item.templateKey));
      if (valid.length > 0) return valid;
      const first = publishedTemplates[0];
      return first
        ? [{ id: crypto.randomUUID(), templateKey: keyOf(first) }]
        : [];
    });
  }, [publishedTemplates]);

  const selectedTemplates = agentSelections
    .map((selection) =>
      publishedTemplates.find((item) => keyOf(item) === selection.templateKey),
    )
    .filter((item): item is AgentTemplateVersionView => Boolean(item));
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
          title="Workflow Templates / 工作流模板"
          actions={
            <Button
              onClick={() => setCreating(true)}
              size="compact"
              variant="primary"
            >
              <Plus aria-hidden="true" size={14} /> New Workflow
            </Button>
          }
        >
          <p className="enterprise-page__lede">
            用 Agent
            Template、触发器、审查节点和输出契约构建可验证、可发布的工作流。
          </p>
          {notice ? (
            <p className="enterprise-page__message is-success" role="status">
              {notice}
            </p>
          ) : null}
          <ol className="enterprise-card-list">
            {snapshot.workflows.map((workflow) => (
              <li key={`${workflow.flowId}@${workflow.version}`}>
                <Workflow aria-hidden="true" size={17} />
                <span>
                  <strong>{workflow.name}</strong>
                  <small>
                    {workflow.flowId}@{workflow.version} ·{" "}
                    {workflow.graph.nodes.length} nodes
                  </small>
                </span>
                <Badge variant="success">published</Badge>
              </li>
            ))}
            {snapshot.workflows.length === 0 ? (
              <li className="enterprise-list__empty">
                尚无已发布 Workflow Template。
              </li>
            ) : null}
          </ol>
        </Panel>
      </div>
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
      selectedTemplates.length === 0 ||
      selectedTemplates.length !== agentSelections.length ||
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
          templates: selectedTemplates,
          requireApproval,
        }),
      );
      setDraft(created);
      setNotice("Workflow 草稿已创建；Agent 模板版本已固定。下一步先验证。 ");
    });
  }

  return (
    <div className="enterprise-page enterprise-workflow-templates">
      <Panel title="Guided workflow builder / 引导式工作流创建">
        <p className="enterprise-page__lede">
          按顺序组合多个 Agent Template，并通过 Dry Run 与真实 Test Run
          后发布为不可变工作流版本。
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
          <WorkflowAgentSequenceEditor
            disabled={publishedTemplates.length === 0}
            onChange={setAgentSelections}
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
              selectedTemplates.length === 0 ||
              selectedTemplates.length !== agentSelections.length ||
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
              void execute("publish", async () => {
                await client.publishFlowDraft(draft!.draft.id, owner.trim());
                await store.load(true);
                setNotice(
                  "Workflow Template 已发布，可前往 Deployments 创建不可变部署快照。 ",
                );
                setCreating(false);
              })
            }
            variant="primary"
          >
            <Send aria-hidden="true" size={14} /> 发布
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
    ["Published", draft.draft.status === "published"],
  ] as const;
  return (
    <ol className="enterprise-progress" aria-label="Workflow 发布进度">
      {steps.map(([label, done]) => (
        <li className={done ? "is-done" : undefined} key={label}>
          <CheckCircle2 aria-hidden="true" size={15} />
          <span>{label}</span>
        </li>
      ))}
    </ol>
  );
}

function keyOf(item: {
  template: { templateId: string; version: number };
}): string {
  return `${item.template.templateId}@${item.template.version}`;
}

function readableError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function isTerminal(status: string): boolean {
  return ["succeeded", "failed", "cancelled"].includes(status);
}
