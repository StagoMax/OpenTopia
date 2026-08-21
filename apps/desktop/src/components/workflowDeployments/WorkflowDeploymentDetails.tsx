import {
  AlertTriangle,
  Bot,
  CheckCircle2,
  CircleStop,
  Inbox,
  Play,
  ShieldCheck,
  Send,
  UserRoundCheck,
  Workflow,
} from "lucide-react";
import { useMemo, useState } from "react";
import type { WorkflowDeployment } from "../../types";
import { Badge, Button, Panel } from "../ui";
import { deploymentStatusLabel, shortHash } from "./model";
import type { WorkflowDeploymentsStore } from "./store";

export function WorkflowDeploymentDetails({
  activeFlowThreadId,
  deployment,
  store,
}: {
  activeFlowThreadId: string | null;
  deployment: WorkflowDeployment;
  store: WorkflowDeploymentsStore;
}) {
  const [inputText, setInputText] = useState("");
  const [advancedInputText, setAdvancedInputText] = useState("");
  const [inputError, setInputError] = useState<string | null>(null);
  const [confirmDisable, setConfirmDisable] = useState(false);
  const workflow = deployment.snapshot.compiledWorkflow;
  const agentSpecs = useMemo(
    () => Object.values(workflow.agentSpecs),
    [workflow.agentSpecs],
  );
  const operationCount = agentSpecs.reduce(
    (count, agent) =>
      count +
      (agent.connectionAuthority.mode === "structured"
        ? agent.connectionAuthority.operations.length
        : 0),
    0,
  );
  const busy = Boolean(store.getSnapshot().busyAction);
  const OutputIcon =
    deployment.snapshot.output.kind === "webhook"
      ? Send
      : deployment.snapshot.output.kind === "human_task"
        ? UserRoundCheck
        : Inbox;

  async function run() {
    if (!activeFlowThreadId) return;
    let input: unknown = inputText.trim() ? { request: inputText.trim() } : {};
    if (advancedInputText.trim()) {
      try {
        input = JSON.parse(advancedInputText);
      } catch {
        setInputError("Advanced Input 必须是有效 JSON。请修正后再运行。");
        return;
      }
    }
    setInputError(null);
    await store.run(activeFlowThreadId, deployment, input);
  }

  return (
    <article className="workflow-deployment-details">
      <header className="workflow-deployment-details__header">
        <span className="workflow-deployment-icon workflow-deployment-icon--large">
          <Workflow aria-hidden="true" size={18} />
        </span>
        <span className="workflow-deployment-details__title">
          <span>
            <h2>{deployment.name}</h2>
            <Badge
              variant={deployment.status === "active" ? "success" : "neutral"}
            >
              {deploymentStatusLabel(deployment.status)}
            </Badge>
          </span>
          <p>
            {workflow.flowId}@{workflow.flowVersion} · {deployment.environment}
          </p>
        </span>
      </header>

      <div className="workflow-deployment-details__grid">
        <Panel title="Deployment Snapshot / 部署快照">
          <dl className="workflow-deployment-detail-list">
            <div>
              <dt>Snapshot ID</dt>
              <dd>
                <code>{deployment.snapshot.id}</code>
              </dd>
            </div>
            <div>
              <dt>Snapshot hash</dt>
              <dd title={deployment.snapshot.contentHash}>
                <code>{shortHash(deployment.snapshot.contentHash)}</code>
              </dd>
            </div>
            <div>
              <dt>Compiled workflow hash</dt>
              <dd title={workflow.contentHash}>
                <code>{shortHash(workflow.contentHash)}</code>
              </dd>
            </div>
            <div>
              <dt>Created by / 创建人</dt>
              <dd>{deployment.createdBy}</dd>
            </div>
          </dl>
        </Panel>

        <Panel title="Runtime contract / 运行契约">
          <div className="workflow-deployment-runtime-contract">
            <span>
              <Play aria-hidden="true" size={16} />
              <strong>{deployment.snapshot.trigger.kind} Trigger</strong>
              <small>快照触发契约</small>
            </span>
            <span>
              <OutputIcon aria-hidden="true" size={16} />
              <strong>{deployment.snapshot.output.kind} Output</strong>
              <small>DeliveryReceipt 追踪</small>
            </span>
            <span>
              <ShieldCheck aria-hidden="true" size={16} />
              <strong>Frozen Authority</strong>
              <small>冻结权限</small>
            </span>
          </div>
        </Panel>
      </div>

      <Panel
        title={`Agent Execution Contexts / Agent 执行上下文 · ${agentSpecs.length}`}
      >
        {agentSpecs.length > 0 ? (
          <div className="workflow-deployment-agents">
            {agentSpecs.map((agent) => {
              const operations =
                agent.connectionAuthority.mode === "structured"
                  ? agent.connectionAuthority.operations.length
                  : 0;
              return (
                <article key={agent.nodeId}>
                  <span className="workflow-deployment-icon">
                    <Bot aria-hidden="true" size={14} />
                  </span>
                  <span>
                    <strong>{agent.name}</strong>
                    <small>
                      {agent.nodeId} · {agent.templateId}@
                      {agent.templateVersion}
                    </small>
                  </span>
                  <Badge variant={operations > 0 ? "info" : "neutral"}>
                    {operations} operations
                  </Badge>
                </article>
              );
            })}
          </div>
        ) : (
          <p className="workflow-deployment-muted">此工作流没有 Agent 节点。</p>
        )}
        <p className="workflow-deployment-summary">
          <CheckCircle2 aria-hidden="true" size={14} /> 共冻结 {operationCount}{" "}
          个 Connection operation；每个节点只看到自己的权限集合。
        </p>
      </Panel>

      {deployment.status === "active" ? (
        <Panel title="Manual Run / 手动运行">
          <div className="workflow-deployment-run-form">
            <label>
              <span>Task input / 任务输入</span>
              <textarea
                aria-describedby={
                  inputError ? "workflow-deployment-input-error" : undefined
                }
                aria-invalid={Boolean(inputError)}
                onChange={(event) => setInputText(event.target.value)}
                placeholder="用自然语言描述本次运行要处理的对象或目标。"
                value={inputText}
              />
            </label>
            <details className="workflow-deployment-advanced-input">
              <summary>Advanced / 使用结构化 JSON 输入</summary>
              <label>
                <span>JSON override / JSON 覆盖</span>
                <textarea
                  onChange={(event) => setAdvancedInputText(event.target.value)}
                  placeholder='例如 {"caseId":"case-1"}'
                  spellCheck={false}
                  value={advancedInputText}
                />
              </label>
            </details>
            {inputError ? (
              <span id="workflow-deployment-input-error" role="alert">
                {inputError}
              </span>
            ) : null}
            {!activeFlowThreadId ? (
              <p className="workflow-deployment-warning">
                <AlertTriangle aria-hidden="true" size={14} /> 先打开或创建一个
                Flow 任务，手动 Run 会挂载到该任务中。
              </p>
            ) : null}
            <Button
              disabled={!activeFlowThreadId || busy}
              onClick={() => void run()}
              variant="primary"
            >
              <Play aria-hidden="true" size={14} /> Run / 运行
            </Button>
          </div>
        </Panel>
      ) : null}

      {deployment.status === "active" ? (
        <section className="workflow-deployment-danger-zone">
          {confirmDisable ? (
            <div role="alert">
              <AlertTriangle aria-hidden="true" size={16} />
              <span>
                <strong>确认停用此 Deployment？</strong>
                <small>新触发会被拒绝；历史 Run 与不可变快照不会删除。</small>
              </span>
              <Button
                onClick={() => setConfirmDisable(false)}
                size="compact"
                variant="quiet"
              >
                取消
              </Button>
              <Button
                disabled={busy}
                onClick={() =>
                  void store
                    .disable(deployment)
                    .then((ok) => ok && setConfirmDisable(false))
                }
                size="compact"
                variant="danger"
              >
                确认停用
              </Button>
            </div>
          ) : (
            <Button onClick={() => setConfirmDisable(true)} variant="danger">
              <CircleStop aria-hidden="true" size={14} /> Disable / 停用部署
            </Button>
          )}
        </section>
      ) : null}
    </article>
  );
}
