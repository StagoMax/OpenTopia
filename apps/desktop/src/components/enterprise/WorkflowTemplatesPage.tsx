import { CheckCircle2, FileJson2, Play, Plus, Send, ShieldCheck, Workflow } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { ApiClient } from "../../api/client";
import type { FlowDraftView } from "../../types";
import { Badge, Button, Panel, Select, Switch, TextField } from "../ui";
import { guidedWorkflowSpec } from "./model";
import { useEnterpriseStore } from "./store";

export function WorkflowTemplatesPage({
  client,
  threadId,
}: {
  client: ApiClient;
  threadId: string | null;
}) {
  const { snapshot, store } = useEnterpriseStore(client);
  const publishedTemplates = useMemo(
    () => snapshot.templates.filter((item) => item.template.status === "published"),
    [snapshot.templates],
  );
  const [flowId, setFlowId] = useState("guided-workflow");
  const [name, setName] = useState("Guided workflow");
  const [owner, setOwner] = useState("local_operator");
  const [outcome, setOutcome] = useState("");
  const [templateKey, setTemplateKey] = useState("");
  const [requireApproval, setRequireApproval] = useState(true);
  const [draft, setDraft] = useState<FlowDraftView | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  useEffect(() => {
    if (publishedTemplates.some((item) => keyOf(item) === templateKey)) return;
    setTemplateKey(publishedTemplates[0] ? keyOf(publishedTemplates[0]) : "");
  }, [publishedTemplates, templateKey]);

  const selectedTemplate =
    publishedTemplates.find((item) => keyOf(item) === templateKey) ?? null;
  const passedTrial = Boolean(
    draft?.trials.some(
      (trial) =>
        trial.draftRevision === draft.draft.revision && trial.status === "passed",
    ),
  );

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
    if (!threadId || !selectedTemplate || !outcome.trim()) return;
    void execute("create", async () => {
      const created = await client.createFlowDraft(
        threadId,
        guidedWorkflowSpec({
          flowId: flowId.trim(),
          name: name.trim(),
          owner: owner.trim(),
          outcome: outcome.trim(),
          template: selectedTemplate,
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
          用业务结果、Agent 模板和人工审查策略创建标准 Agent → Review → Output 流程；常规路径不需要编辑 JSON。
        </p>
        {!threadId ? (
          <p className="enterprise-page__message is-warning" role="status">
            请先新建或选择一个 Flow 任务，用于保存草稿与验证记录。
          </p>
        ) : null}
        <div className="enterprise-form-grid">
          <TextField label="Workflow ID" onChange={(event) => setFlowId(event.target.value)} value={flowId} />
          <TextField label="名称" onChange={(event) => setName(event.target.value)} value={name} />
          <TextField label="所有者" onChange={(event) => setOwner(event.target.value)} value={owner} />
          <label className="enterprise-field enterprise-field--wide">
            <span>用自然语言描述希望完成的业务结果</span>
            <textarea
              onChange={(event) => setOutcome(event.target.value)}
              placeholder="例如：读取新客户资料，检查必填字段并生成一份可供销售复核的摘要。"
              rows={4}
              value={outcome}
            />
          </label>
          <label className="enterprise-field enterprise-field--wide">
            <span>执行 Agent 模板</span>
            <Select
              disabled={publishedTemplates.length === 0}
              label="执行 Agent 模板"
              onChange={setTemplateKey}
              options={publishedTemplates.map((item) => ({
                value: keyOf(item),
                label: `${item.template.name} · ${item.template.templateId}@${item.template.version}`,
              }))}
              value={templateKey}
            />
            {publishedTemplates.length === 0 ? <small>请先在 Agents 发布一个模板版本。</small> : null}
          </label>
          <label className="enterprise-switch enterprise-field--wide">
            <span>
              <strong>执行后人工审查</strong>
              <small>在 Agent 输出与 Inbox Output 之间增加 Approval 节点。</small>
            </span>
            <Switch checked={requireApproval} onChange={setRequireApproval} />
          </label>
        </div>
        <div className="enterprise-actions">
          <Button
            disabled={!threadId || !selectedTemplate || !outcome.trim() || Boolean(busy)}
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
                setNotice("静态验证完成。 ");
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
                setDraft((await client.listFlowDrafts(threadId!)).find((item) => item.draft.id === draft!.draft.id) ?? draft);
                setNotice("确定性 Trial 已通过。 ");
              })
            }
          >
            <Play aria-hidden="true" size={14} /> Trial
          </Button>
          <Button
            disabled={!draft?.draft.lastValidation?.valid || !passedTrial || Boolean(busy)}
            onClick={() =>
              void execute("publish", async () => {
                await client.publishFlowDraft(draft!.draft.id, owner.trim());
                await store.load(true);
                setNotice("Workflow Template 已发布，可前往 Deployments 创建不可变部署快照。 ");
              })
            }
            variant="primary"
          >
            <Send aria-hidden="true" size={14} /> 发布
          </Button>
        </div>
        {draft ? <WorkflowProgress draft={draft} passedTrial={passedTrial} /> : null}
        {error ? <p className="enterprise-page__message is-error" role="alert">{error}</p> : null}
        {notice ? <p className="enterprise-page__message is-success" role="status">{notice}</p> : null}
        {draft ? (
          <details className="enterprise-advanced">
            <summary><FileJson2 aria-hidden="true" size={15} /> Advanced / 高级 JSON 检查</summary>
            <pre>{JSON.stringify(draft.draft.spec, null, 2)}</pre>
          </details>
        ) : null}
      </Panel>

      <Panel title="Published workflow templates / 已发布工作流模板">
        <ol className="enterprise-card-list">
          {snapshot.workflows.map((workflow) => (
            <li key={`${workflow.flowId}@${workflow.version}`}>
              <Workflow aria-hidden="true" size={17} />
              <span>
                <strong>{workflow.name}</strong>
                <small>{workflow.flowId}@{workflow.version} · {workflow.graph.nodes.length} nodes</small>
              </span>
              <Badge variant="success">published</Badge>
            </li>
          ))}
          {snapshot.workflows.length === 0 ? <li className="enterprise-list__empty">尚无已发布 Workflow Template。</li> : null}
        </ol>
      </Panel>
    </div>
  );
}

function WorkflowProgress({ draft, passedTrial }: { draft: FlowDraftView; passedTrial: boolean }) {
  const steps = [
    ["Draft", true],
    ["Validated", Boolean(draft.draft.lastValidation?.valid)],
    ["Trial", passedTrial],
    ["Published", draft.draft.status === "published"],
  ] as const;
  return (
    <ol className="enterprise-progress" aria-label="Workflow 发布进度">
      {steps.map(([label, done]) => (
        <li className={done ? "is-done" : undefined} key={label}>
          <CheckCircle2 aria-hidden="true" size={15} /><span>{label}</span>
        </li>
      ))}
    </ol>
  );
}

function keyOf(item: { template: { templateId: string; version: number } }): string {
  return `${item.template.templateId}@${item.template.version}`;
}

function readableError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
