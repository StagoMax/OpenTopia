import { AlertTriangle, LoaderCircle, RefreshCw, Rocket } from "lucide-react";
import type { ApiClient } from "../../api/client";
import { Button, IconButton } from "../ui";
import { DeploymentCollection } from "./DeploymentCollection";
import { WorkflowDeploymentDetails } from "./WorkflowDeploymentDetails";
import { WorkflowDeploymentEditor } from "./WorkflowDeploymentEditor";
import { useWorkflowDeploymentsStore } from "./store";
import "../../styles/workflow-deployments.css";

export function WorkflowDeploymentsPanel({
  activeFlowThreadId,
  client,
}: {
  activeFlowThreadId: string | null;
  client: ApiClient;
}) {
  const { snapshot, store } = useWorkflowDeploymentsStore(client);
  const selected = snapshot.deployments.find(
    (deployment) => deployment.id === snapshot.selectedDeploymentId,
  );

  if (snapshot.status === "loading" || snapshot.status === "idle") {
    return (
      <div className="workflow-deployment-page-state" role="status">
        <LoaderCircle
          className="workflow-deployment-spin"
          aria-hidden="true"
          size={18}
        />
        <strong>正在加载 Deployments</strong>
        <span>读取已发布 Flow 和不可变部署快照…</span>
      </div>
    );
  }

  if (snapshot.status === "error" && snapshot.deployments.length === 0) {
    return (
      <div className="workflow-deployment-page-state" role="alert">
        <AlertTriangle aria-hidden="true" size={18} />
        <strong>Deployments 加载失败</strong>
        <span>{snapshot.error}</span>
        <Button onClick={() => void store.load(true)} variant="primary">
          <RefreshCw aria-hidden="true" size={14} /> 重试
        </Button>
      </div>
    );
  }

  if (snapshot.editorOpen) {
    return (
      <div className="workflow-deployments-workspace workflow-deployments-workspace--editor">
        <WorkflowDeploymentEditor snapshot={snapshot} store={store} />
      </div>
    );
  }

  return (
    <div className="workflow-deployments-workspace">
      <aside className="workflow-deployments-workspace__collection">
        <DeploymentCollection snapshot={snapshot} store={store} />
      </aside>
      <main className="workflow-deployments-workspace__detail">
        <div className="workflow-deployments-workspace__toolbar">
          <span>
            <strong>Deployment control plane / 部署控制面</strong>
            <small>编译、冻结、触发与停用</small>
          </span>
          <IconButton
            aria-label="刷新 Deployments"
            disabled={Boolean(snapshot.busyAction)}
            onClick={() => void store.load(true)}
            size="compact"
          >
            <RefreshCw aria-hidden="true" size={14} />
          </IconButton>
        </div>
        {snapshot.error || snapshot.notice ? (
          <div
            className={`workflow-deployment-feedback${snapshot.error ? " workflow-deployment-feedback--error" : " workflow-deployment-feedback--success"}`}
            role={snapshot.error ? "alert" : "status"}
          >
            <span>{snapshot.error ?? snapshot.notice}</span>
            <Button
              onClick={() => store.clearFeedback()}
              size="compact"
              variant="quiet"
            >
              关闭
            </Button>
          </div>
        ) : null}
        {selected ? (
          <WorkflowDeploymentDetails
            activeFlowThreadId={activeFlowThreadId}
            deployment={selected}
            store={store}
          />
        ) : (
          <div className="workflow-deployment-empty-state">
            <Rocket aria-hidden="true" size={20} />
            <strong>创建第一个 Deployment</strong>
            <span>
              发布只是定义版本；Deployment 会把 Flow 与每个 Agent 节点的模板和
              Connection 权限编译成不可变快照。
            </span>
            <Button onClick={() => store.beginCreate()} variant="primary">
              <Rocket aria-hidden="true" size={14} /> Create Deployment /
              创建部署
            </Button>
          </div>
        )}
      </main>
    </div>
  );
}
