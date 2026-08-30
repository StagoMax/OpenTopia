import {
  AlertTriangle,
  Cable,
  LoaderCircle,
  Pencil,
  Plus,
  RefreshCw,
  RotateCw,
  Save,
} from "lucide-react";
import type { ApiClient } from "../../api/client";
import { Button, IconButton } from "../ui";
import {
  FlowInspectorPanel,
  FlowInspectorSection,
} from "../enterprise/FlowInspectorPanel";
import {
  FlowInspectorPortal,
  useFlowWorkspaceTitle,
} from "../enterprise/flowAgentSelection";
import {
  useEnterpriseSubpageHeader,
  type EnterprisePageHeaderChange,
} from "../enterprise/pageHeader";
import { ConnectionDetails } from "./ConnectionDetails";
import { ConnectionEditor } from "./ConnectionEditor";
import {
  connectionAccountLabel,
  connectionStatusLabel,
  connectionStatusVariant,
  definitionForConnection,
} from "./model";
import { useConnectionsStore } from "./store";
import "../../styles/connections.css";

const editorFormId = "flow-connection-editor";

export function ConnectionsWorkspacePanel({
  client,
  onPageHeaderChange,
}: {
  client: ApiClient;
  onPageHeaderChange?: EnterprisePageHeaderChange;
}) {
  const { snapshot, store } = useConnectionsStore(client);
  const selected = snapshot.connections.find(
    (connection) => connection.id === snapshot.selectedConnectionId,
  );
  const editorTitle =
    snapshot.editorMode === "edit"
      ? `Edit ${selected?.name ?? "Connection"}`
      : "New Connection / 新建 Connection";
  useFlowWorkspaceTitle(snapshot.editorMode ? editorTitle : selected?.name);
  useEnterpriseSubpageHeader(onPageHeaderChange, Boolean(snapshot.editorMode), {
    title:
      snapshot.editorMode === "edit"
        ? "Connections / 编辑 Connection"
        : "Connections / 创建 Connection",
    backLabel: "返回 Connections",
    onBack: () => store.cancelEdit(),
  });

  if (snapshot.status === "loading" || snapshot.status === "idle") {
    return (
      <div className="connections-page-state" role="status">
        <LoaderCircle
          className="connections-spin"
          aria-hidden="true"
          size={18}
        />
        <strong>正在加载 Connections</strong>
        <span>读取 Provider、账号连接和 capability revision…</span>
      </div>
    );
  }

  if (snapshot.status === "error" && snapshot.connections.length === 0) {
    return (
      <div className="connections-page-state" role="alert">
        <AlertTriangle aria-hidden="true" size={18} />
        <strong>Connections 加载失败</strong>
        <span>{snapshot.error}</span>
        <Button onClick={() => void store.load(true)} variant="primary">
          <RefreshCw aria-hidden="true" size={14} /> 重试
        </Button>
      </div>
    );
  }

  if (snapshot.editorMode) {
    const saving =
      snapshot.busyAction?.startsWith("save:") ||
      snapshot.busyAction === "create";
    return (
      <div className="connections-workspace connections-workspace--editor">
        <div className="connections-editor-stage">
          <Cable aria-hidden="true" size={28} />
          <strong>{editorTitle}</strong>
          <span>
            在右侧配置账号、租户、认证上下文与独立
            runtime。保存后会在左侧列表中选中该 Connection。
          </span>
        </div>
        <FlowInspectorPortal>
          <FlowInspectorPanel
            actions={
              <>
                <Button
                  disabled={Boolean(saving)}
                  onClick={() => store.cancelEdit()}
                  size="compact"
                  variant="quiet"
                >
                  取消
                </Button>
                <Button
                  disabled={Boolean(saving)}
                  form={editorFormId}
                  size="compact"
                  type="submit"
                  variant="primary"
                >
                  <Save aria-hidden="true" size={14} />
                  {saving ? "保存中…" : "保存"}
                </Button>
              </>
            }
            status="draft"
            statusVariant="warning"
            title={
              snapshot.editorMode === "edit"
                ? "Edit connection"
                : "New connection"
            }
          >
            <ConnectionEditor
              formId={editorFormId}
              snapshot={snapshot}
              store={store}
              submitAction="external"
            />
          </FlowInspectorPanel>
        </FlowInspectorPortal>
      </div>
    );
  }

  return (
    <div className="connections-workspace">
      <main className="connections-workspace__detail">
        {selected ? (
          <ConnectionDetails
            connection={selected}
            definition={definitionForConnection(snapshot.definitions, selected)}
            snapshot={snapshot}
            store={store}
            variant="core"
          />
        ) : (
          <div className="connections-empty-state">
            <Cable aria-hidden="true" size={20} />
            <strong>创建第一个 Connection</strong>
            <span>
              一个 Connection 代表某个 Provider 下的具体账号、租户和独立
              runtime。Agent Template 只引用它，不复制凭据。
            </span>
          </div>
        )}
      </main>

      <FlowInspectorPortal>
        {selected ? (
          <FlowInspectorPanel
            actions={
              <>
                <IconButton
                  aria-label="编辑 Connection"
                  disabled={Boolean(snapshot.busyAction)}
                  onClick={() => store.beginEdit()}
                  size="compact"
                >
                  <Pencil aria-hidden="true" size={14} />
                </IconButton>
                <IconButton
                  aria-label="测试 Connection"
                  disabled={Boolean(snapshot.busyAction) || !selected.enabled}
                  onClick={() => void store.test(selected.id)}
                  size="compact"
                >
                  <RotateCw aria-hidden="true" size={14} />
                </IconButton>
                <IconButton
                  aria-label="刷新 Connection 能力"
                  disabled={
                    Boolean(snapshot.busyAction) || selected.status !== "ready"
                  }
                  onClick={() => void store.refreshCapabilities(selected.id)}
                  size="compact"
                >
                  <RefreshCw aria-hidden="true" size={14} />
                </IconButton>
              </>
            }
            status={connectionStatusLabel(selected.status)}
            statusVariant={connectionStatusVariant(selected.status)}
            subtitle={connectionAccountLabel(selected)}
            title="Connection"
          >
            <ConnectionDetails
              connection={selected}
              definition={definitionForConnection(
                snapshot.definitions,
                selected,
              )}
              snapshot={snapshot}
              store={store}
              variant="inspector"
            />
          </FlowInspectorPanel>
        ) : (
          <FlowInspectorPanel
            actions={
              <Button
                onClick={() => store.beginCreate()}
                size="compact"
                variant="primary"
              >
                <Plus aria-hidden="true" size={14} /> 新建
              </Button>
            }
            status="empty"
            title="Connections"
          >
            <FlowInspectorSection title="Configuration / 配置">
              <p>
                创建 Connection
                后，可在这里管理账号、runtime、健康状态与能力快照。
              </p>
            </FlowInspectorSection>
          </FlowInspectorPanel>
        )}
      </FlowInspectorPortal>
    </div>
  );
}
