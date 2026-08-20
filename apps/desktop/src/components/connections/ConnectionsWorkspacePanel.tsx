import {
  AlertTriangle,
  Cable,
  LoaderCircle,
  Plus,
  RefreshCw,
} from "lucide-react";
import type { ApiClient } from "../../api/client";
import { Button, IconButton } from "../ui";
import { ConnectionCollection } from "./ConnectionCollection";
import { ConnectionDetails } from "./ConnectionDetails";
import { ConnectionEditor } from "./ConnectionEditor";
import { definitionForConnection } from "./model";
import { useConnectionsStore } from "./store";
import "../../styles/connections.css";

export function ConnectionsWorkspacePanel({ client }: { client: ApiClient }) {
  const { snapshot, store } = useConnectionsStore(client);
  const selected = snapshot.connections.find(
    (connection) => connection.id === snapshot.selectedConnectionId,
  );

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
    return (
      <div className="connections-workspace connections-workspace--editor">
        <ConnectionEditor snapshot={snapshot} store={store} />
      </div>
    );
  }

  return (
    <div className="connections-workspace">
      <aside className="connections-workspace__collection">
        <ConnectionCollection snapshot={snapshot} store={store} />
      </aside>
      <main className="connections-workspace__detail">
        <div className="connections-workspace__toolbar">
          <span>
            <strong>Connection control plane</strong>
            <small>账号、租户、runtime、健康与不可变能力快照</small>
          </span>
          <IconButton
            aria-label="刷新 Connections"
            disabled={Boolean(snapshot.busyAction)}
            onClick={() => void store.load(true)}
            size="compact"
          >
            <RefreshCw aria-hidden="true" size={14} />
          </IconButton>
        </div>
        {selected ? (
          <ConnectionDetails
            connection={selected}
            definition={definitionForConnection(snapshot.definitions, selected)}
            snapshot={snapshot}
            store={store}
          />
        ) : (
          <div className="connections-empty-state">
            <Cable aria-hidden="true" size={20} />
            <strong>创建第一个 Connection</strong>
            <span>
              一个 Connection 代表某个 Provider 下的具体账号、租户和独立
              runtime。Agent Template 只引用它，不复制凭据。
            </span>
            <Button onClick={() => store.beginCreate()} variant="primary">
              <Plus aria-hidden="true" size={14} /> 新建 Connection
            </Button>
          </div>
        )}
      </main>
    </div>
  );
}
