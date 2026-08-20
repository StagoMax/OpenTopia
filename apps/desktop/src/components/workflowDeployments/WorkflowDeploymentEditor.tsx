import { ArrowLeft, Boxes, LoaderCircle } from "lucide-react";
import { useMemo, useState, type FormEvent } from "react";
import { Button, Panel, Select, TextField } from "../ui";
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
  const busy = snapshot.busyAction === "create";
  const canSubmit = Boolean(
    selectedDefinition && name.trim() && environment.trim() && createdBy.trim(),
  );

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!selectedDefinition || !canSubmit || busy) return;
    await store.create({
      flowId: selectedDefinition.flowId,
      flowVersion: selectedDefinition.version,
      name: name.trim(),
      environment: environment.trim(),
      createdBy: createdBy.trim(),
    });
  }

  return (
    <form
      className="workflow-deployment-editor"
      onSubmit={(event) => void submit(event)}
    >
      <header className="workflow-deployment-editor__header">
        <Button
          onClick={() => store.cancelCreate()}
          size="compact"
          variant="quiet"
        >
          <ArrowLeft aria-hidden="true" size={14} /> 返回
        </Button>
        <span>
          <h2>Create Deployment / 创建部署</h2>
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
            <label className="workflow-deployment-select-field">
              <span>Published Flow / 已发布 Flow</span>
              <Select
                label="Published Flow / 已发布 Flow"
                onChange={setDefinitionId}
                options={snapshot.definitions.map((definition) => ({
                  value: definition.id,
                  label: `${definition.name} · ${definition.flowId}@${definition.version}`,
                }))}
                value={definitionId}
              />
            </label>
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
            <span>Trigger: Manual / 手动 · Output: Inbox / 收件箱</span>
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
