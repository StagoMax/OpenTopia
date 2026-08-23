import { Boxes, LoaderCircle } from "lucide-react";
import { useMemo, useState, type FormEvent } from "react";
import { Button, Panel, SelectField, TextField } from "../ui";
import type {
  WorkflowOutput,
  WorkflowOutputReviewPolicy,
} from "../../types";
import type {
  WorkflowDeploymentsSnapshot,
  WorkflowDeploymentsStore,
} from "./store";

export function WorkflowDeploymentEditor({
  snapshot,
  store,
}: {
  snapshot: WorkflowDeploymentsSnapshot;
  store: WorkflowDeploymentsStore;
}) {
  const firstDefinitionId = snapshot.definitions[0]?.id ?? "";
  const [definitionId, setDefinitionId] = useState(firstDefinitionId);
  const selectedDefinition = useMemo(
    () =>
      snapshot.definitions.find(
        (definition) => definition.id === definitionId,
      ) ?? null,
    [definitionId, snapshot.definitions],
  );
  const [name, setName] = useState(
    snapshot.definitions[0] ? `${snapshot.definitions[0].name} Production` : "",
  );
  const [environment, setEnvironment] = useState("production");
  const [createdBy, setCreatedBy] = useState("local-user");
  const [outputKind, setOutputKind] = useState<WorkflowOutput["kind"]>("inbox");
  const [outputReviewPolicy, setOutputReviewPolicy] =
    useState<WorkflowOutputReviewPolicy>("explicit_nodes_only");
  const [webhookEndpoint, setWebhookEndpoint] = useState("");
  const [webhookCredentialRef, setWebhookCredentialRef] = useState("");
  const [humanTaskTitle, setHumanTaskTitle] = useState(
    "Review downstream delivery",
  );
  const [humanTaskDescription, setHumanTaskDescription] = useState(
    "确认 Flow 输出已被下游业务流程接收。",
  );
  const [humanTaskAssignee, setHumanTaskAssignee] = useState("");
  const busy = snapshot.busyAction === "create";
  const outputReady =
    outputKind === "inbox" ||
    (outputKind === "webhook" && webhookEndpoint.trim()) ||
    (outputKind === "human_task" &&
      humanTaskTitle.trim() &&
      humanTaskDescription.trim());
  const canSubmit = Boolean(
    selectedDefinition &&
    name.trim() &&
    environment.trim() &&
    createdBy.trim() &&
    outputReady,
  );

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!selectedDefinition || !canSubmit || busy) return;
    const output: WorkflowOutput =
      outputKind === "webhook"
        ? {
            kind: "webhook",
            endpoint: webhookEndpoint.trim(),
            credentialRef: webhookCredentialRef.trim() || undefined,
          }
        : outputKind === "human_task"
          ? {
              kind: "human_task",
              title: humanTaskTitle.trim(),
              description: humanTaskDescription.trim(),
              assignedTo: humanTaskAssignee.trim() || undefined,
            }
          : { kind: "inbox" };
    await store.create({
      flowId: selectedDefinition.flowId,
      flowVersion: selectedDefinition.version,
      name: name.trim(),
      environment: environment.trim(),
      createdBy: createdBy.trim(),
      output,
      outputReviewPolicy,
    });
  }

  return (
    <form
      className="workflow-deployment-editor"
      onSubmit={(event) => void submit(event)}
    >
      <header className="workflow-deployment-editor__header">
        <span>
          <h2>Deployment Snapshot / 部署快照</h2>
          <p>
            Workflow Compiler / 工作流编译器会冻结 Flow、Agent Template 与
            operation-level Connection 权限。
          </p>
        </span>
      </header>

      {snapshot.error ? (
        <div
          className="workflow-deployment-feedback workflow-deployment-feedback--error"
          role="alert"
        >
          <span>{snapshot.error}</span>
          <Button
            onClick={() => store.clearFeedback()}
            size="compact"
            variant="quiet"
          >
            关闭
          </Button>
        </div>
      ) : null}

      {snapshot.definitions.length === 0 ? (
        <div className="workflow-deployment-empty-state">
          <Boxes aria-hidden="true" size={20} />
          <strong>没有可部署的 Published Flow</strong>
          <span>
            先在 Flow Template 中验证并发布一个版本，再回到这里创建部署。
          </span>
        </div>
      ) : (
        <>
          <section className="workflow-deployment-form-section">
            <header>
              <h3>Source / 来源</h3>
              <p>部署引用已发布的精确版本，不会自动跟随后续模板变更。</p>
            </header>
            <SelectField
              fieldClassName="workflow-deployment-select-field"
              label="Published Flow / 已发布 Flow"
              onChange={setDefinitionId}
              options={snapshot.definitions.map((definition) => ({
                value: definition.id,
                label: `${definition.name} · ${definition.flowId}@${definition.version}`,
              }))}
              value={definitionId}
            />
            {selectedDefinition ? (
              <Panel title="Compile preview / 编译预览">
                <dl className="workflow-deployment-detail-list">
                  <div>
                    <dt>Definition</dt>
                    <dd>
                      {selectedDefinition.flowId}@{selectedDefinition.version}
                    </dd>
                  </div>
                  <div>
                    <dt>Agent nodes / Agent 节点</dt>
                    <dd>
                      {
                        selectedDefinition.graph.nodes.filter(
                          (node) => node.kind === "agent",
                        ).length
                      }
                    </dd>
                  </div>
                  <div>
                    <dt>Risk class / 风险等级</dt>
                    <dd>{selectedDefinition.riskClass}</dd>
                  </div>
                </dl>
              </Panel>
            ) : null}
          </section>

          <section className="workflow-deployment-form-section">
            <header>
              <h3>Output contract / 输出契约</h3>
              <p>
                输出目标冻结在 Deployment Snapshot；投递状态写入
                DeliveryReceipt。
              </p>
            </header>
            <SelectField
              fieldClassName="workflow-deployment-select-field"
              label="Output / 输出"
              onChange={(value) =>
                setOutputKind(value as WorkflowOutput["kind"])
              }
              options={[
                { value: "inbox", label: "Inbox / 运行记录" },
                { value: "webhook", label: "Webhook / 外部接口" },
                { value: "human_task", label: "HumanTask / 人工交接" },
              ]}
              value={outputKind}
            />
            <SelectField
              fieldClassName="workflow-deployment-select-field"
              hint="工作流中的 Approval 节点始终生效；这里仅控制最终输出是否额外复核。"
              label="Output review / 最终输出复核"
              onChange={(value) =>
                setOutputReviewPolicy(value as WorkflowOutputReviewPolicy)
              }
              options={[
                {
                  value: "explicit_nodes_only",
                  label: "仅显式 Approval 节点（推荐）",
                },
                {
                  value: "always_review_output",
                  label: "每次最终输出都人工复核",
                },
              ]}
              value={outputReviewPolicy}
            />
            {outputKind === "webhook" ? (
              <div className="workflow-deployment-form-grid">
                <TextField
                  hint="公网目标必须使用 HTTPS；本地测试允许 loopback HTTP"
                  label="Endpoint / 接口地址"
                  onChange={(event) => setWebhookEndpoint(event.target.value)}
                  required
                  value={webhookEndpoint}
                />
                <TextField
                  hint="只保存引用，例如 env:FLOW_OUTPUT_TOKEN"
                  label="Credential ref / 凭据引用"
                  onChange={(event) =>
                    setWebhookCredentialRef(event.target.value)
                  }
                  value={webhookCredentialRef}
                />
              </div>
            ) : null}
            {outputKind === "human_task" ? (
              <div className="workflow-deployment-form-grid">
                <TextField
                  label="Task title / 任务标题"
                  onChange={(event) => setHumanTaskTitle(event.target.value)}
                  required
                  value={humanTaskTitle}
                />
                <TextField
                  label="Description / 说明"
                  onChange={(event) =>
                    setHumanTaskDescription(event.target.value)
                  }
                  required
                  value={humanTaskDescription}
                />
                <TextField
                  hint="可留空，由 Inbox 中的操作员领取"
                  label="Assignee / 负责人"
                  onChange={(event) => setHumanTaskAssignee(event.target.value)}
                  value={humanTaskAssignee}
                />
              </div>
            ) : null}
          </section>

          <section className="workflow-deployment-form-section">
            <header>
              <h3>Identity / 部署标识</h3>
              <p>环境是部署标签；本阶段不会自动连接外部 CI/CD 环境。</p>
            </header>
            <div className="workflow-deployment-form-grid">
              <TextField
                label="Name / 名称"
                onChange={(event) => setName(event.target.value)}
                required
                value={name}
              />
              <TextField
                hint="例如 production、staging 或 finance-prod"
                label="Environment / 环境"
                onChange={(event) => setEnvironment(event.target.value)}
                required
                value={environment}
              />
              <TextField
                hint="写入不可变快照的审计字段"
                label="Created by / 创建人"
                onChange={(event) => setCreatedBy(event.target.value)}
                required
                value={createdBy}
              />
            </div>
          </section>

          <footer className="workflow-deployment-editor__footer">
            <span>
              Trigger: Release Channel / 发布通道 · Output: {outputKind}
            </span>
            <Button
              disabled={!canSubmit || busy}
              type="submit"
              variant="primary"
            >
              {busy ? (
                <LoaderCircle
                  className="workflow-deployment-spin"
                  aria-hidden="true"
                  size={14}
                />
              ) : (
                <Boxes aria-hidden="true" size={14} />
              )}
              {busy ? "正在编译…" : "Compile & deploy / 编译并部署"}
            </Button>
          </footer>
        </>
      )}
    </form>
  );
}
