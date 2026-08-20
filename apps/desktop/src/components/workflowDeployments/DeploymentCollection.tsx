import { CircleAlert, Rocket, Search } from "lucide-react";
import { useMemo, useState } from "react";
import type { ApiClient } from "../../api/client";
import { Badge, Button, TextField } from "../ui";
import { deploymentMatchesQuery, deploymentStatusLabel } from "./model";
import {
  useWorkflowDeploymentsStore,
  type WorkflowDeploymentsSnapshot,
  type WorkflowDeploymentsStore,
} from "./store";

export function DeploymentCollection({
  compact = false,
  snapshot,
  store,
}: {
  compact?: boolean;
  snapshot: WorkflowDeploymentsSnapshot;
  store: WorkflowDeploymentsStore;
}) {
  const [query, setQuery] = useState("");
  const deployments = useMemo(
    () =>
      snapshot.deployments.filter((deployment) =>
        deploymentMatchesQuery(deployment, query),
      ),
    [query, snapshot.deployments],
  );

  return (
    <section
      aria-label="Deployments / 部署"
      className={`workflow-deployment-collection${compact ? " workflow-deployment-collection--compact" : ""}`}
    >
      <header className="workflow-deployment-collection__header">
        <span>
          <strong>Deployments</strong>
          <small>{snapshot.deployments.length} 个不可变部署</small>
        </span>
        <Button
          onClick={() => store.beginCreate()}
          size="compact"
          variant="quiet"
        >
          <Rocket aria-hidden="true" size={14} /> 部署
        </Button>
      </header>
      {!compact && snapshot.deployments.length > 4 ? (
        <TextField
          aria-label="搜索 Deployments"
          label={
            <span className="workflow-deployment-search-label">
              <Search aria-hidden="true" size={14} /> 搜索
            </span>
          }
          onChange={(event) => setQuery(event.target.value)}
          placeholder="名称、Flow 或环境"
          value={query}
        />
      ) : null}
      <div className="workflow-deployment-collection__list">
        {snapshot.status === "loading" ? (
          <div className="workflow-deployment-inline-state" role="status">
            正在加载部署…
          </div>
        ) : null}
        {deployments.map((deployment) => {
          const workflow = deployment.snapshot.compiledWorkflow;
          const selected = deployment.id === snapshot.selectedDeploymentId;
          return (
            <button
              aria-current={selected ? "page" : undefined}
              className={`workflow-deployment-collection__item${selected ? " is-selected" : ""}`}
              key={deployment.id}
              onClick={() => store.select(deployment.id)}
              type="button"
            >
              <span className="workflow-deployment-icon">
                <Rocket aria-hidden="true" size={14} />
              </span>
              <span>
                <strong>{deployment.name}</strong>
                <small>
                  {workflow.flowId}@{workflow.flowVersion} ·{" "}
                  {deployment.environment}
                </small>
              </span>
              <Badge
                variant={deployment.status === "active" ? "success" : "neutral"}
              >
                {compact
                  ? deployment.status === "active"
                    ? "Active"
                    : "Disabled"
                  : deploymentStatusLabel(deployment.status)}
              </Badge>
            </button>
          );
        })}
      </div>
      {snapshot.status !== "loading" && deployments.length === 0 ? (
        <div className="workflow-deployment-collection__empty">
          <CircleAlert aria-hidden="true" size={16} />
          <span>
            {snapshot.deployments.length > 0
              ? "没有匹配的 Deployment"
              : "尚未创建 Deployment"}
          </span>
        </div>
      ) : null}
    </section>
  );
}

export function WorkflowDeploymentSidebarCollection({
  client,
}: {
  client: ApiClient;
}) {
  const { snapshot, store } = useWorkflowDeploymentsStore(client);
  return (
    <div className="workflow-deployment-sidebar-collection">
      <DeploymentCollection compact snapshot={snapshot} store={store} />
      {snapshot.status === "error" ? (
        <Button
          onClick={() => void store.load(true)}
          size="compact"
          variant="quiet"
        >
          重试加载
        </Button>
      ) : null}
    </div>
  );
}
