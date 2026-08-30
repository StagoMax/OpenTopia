import { Cable, CircleAlert, Plus, Search } from "lucide-react";
import { useMemo, useState } from "react";
import type { ApiClient } from "../../api/client";
import type { ConnectionStatus } from "../../types";
import {
  Badge,
  Button,
  IconButton,
  SidebarRow,
  TextField,
  type SidebarRowStatusTone,
} from "../ui";
import {
  connectionAccountLabel,
  connectionStatusLabel,
  connectionStatusVariant,
  definitionForConnection,
  integrationKindLabel,
} from "./model";
import {
  useConnectionsStore,
  type ConnectionsSnapshot,
  type ConnectionsStore,
} from "./store";

export type ConnectionCollectionProps = {
  compact?: boolean;
  snapshot: ConnectionsSnapshot;
  store: ConnectionsStore;
};

export function ConnectionCollection({
  compact = false,
  snapshot,
  store,
}: ConnectionCollectionProps) {
  const [query, setQuery] = useState("");
  const filtered = useMemo(() => {
    const normalized = query.trim().toLocaleLowerCase();
    if (!normalized) return snapshot.connections;
    return snapshot.connections.filter((connection) => {
      const definition = definitionForConnection(
        snapshot.definitions,
        connection,
      );
      return [
        connection.name,
        connection.environment,
        connectionAccountLabel(connection),
        definition?.name ?? "",
      ].some((value) => value.toLocaleLowerCase().includes(normalized));
    });
  }, [query, snapshot.connections, snapshot.definitions]);

  return (
    <section
      className={`connection-collection${compact ? " connection-collection--compact" : ""}`}
      aria-label="Connections"
    >
      <header className="connection-collection__header">
        <span>
          <strong>Connections</strong>
          <small>{snapshot.connections.length} 个账号连接</small>
        </span>
        {compact ? (
          <IconButton
            aria-label="新建 Connection"
            onClick={() => store.beginCreate()}
            size="compact"
            title="新建 Connection"
          >
            <Plus aria-hidden="true" size={14} />
          </IconButton>
        ) : (
          <Button
            onClick={() => store.beginCreate()}
            size="compact"
            variant="quiet"
          >
            <Plus aria-hidden="true" size={14} /> 新建
          </Button>
        )}
      </header>
      {!compact && snapshot.connections.length > 4 ? (
        <TextField
          aria-label="搜索 Connections"
          label={
            <span className="connection-collection__search-label">
              <Search aria-hidden="true" size={14} /> 搜索
            </span>
          }
          placeholder="名称、Provider、账号或环境"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />
      ) : null}
      <div className="connection-collection__list">
        {snapshot.status === "loading" ? (
          <div className="connections-inline-state" role="status">
            正在加载账号连接…
          </div>
        ) : null}
        {filtered.map((connection) => {
          const definition = definitionForConnection(
            snapshot.definitions,
            connection,
          );
          const selected = connection.id === snapshot.selectedConnectionId;
          return compact ? (
            <SidebarRow
              active={selected}
              className="connection-collection__sidebar-row"
              description={`${definition ? integrationKindLabel(definition.kind) : "Unknown"} · ${connection.environment}`}
              key={connection.id}
              onSelect={() => store.select(connection.id)}
              status={{
                label: connectionStatusLabel(connection.status),
                tone: connectionStatusTone(connection.status),
              }}
              title={`${connection.name} · ${definition ? integrationKindLabel(definition.kind) : "Unknown"} · ${connection.environment}`}
            />
          ) : (
            <button
              aria-current={selected ? "page" : undefined}
              className={`connection-collection__item${selected ? " is-selected" : ""}`}
              key={connection.id}
              onClick={() => store.select(connection.id)}
              type="button"
            >
              <span className="connections-icon">
                <Cable aria-hidden="true" size={14} />
              </span>
              <span>
                <strong>{connection.name}</strong>
                <small>
                  {definition
                    ? integrationKindLabel(definition.kind)
                    : "Unknown"}{" "}
                  · {connection.environment}
                </small>
                {!compact ? (
                  <small>{connectionAccountLabel(connection)}</small>
                ) : null}
              </span>
              <Badge variant={connectionStatusVariant(connection.status)}>
                {connectionStatusLabel(connection.status)}
              </Badge>
            </button>
          );
        })}
      </div>
      {snapshot.status !== "loading" && filtered.length === 0 ? (
        <div className="connection-collection__empty">
          <CircleAlert aria-hidden="true" size={16} />
          <span>
            {snapshot.connections.length > 0
              ? "没有匹配的 Connection"
              : "尚未创建 Connection"}
          </span>
        </div>
      ) : null}
    </section>
  );
}

function connectionStatusTone(status: ConnectionStatus): SidebarRowStatusTone {
  if (status === "ready") return "success";
  if (status === "reauth_required") return "danger";
  if (status === "degraded") return "warning";
  if (status === "configured") return "info";
  return "neutral";
}

export function ConnectionSidebarCollection({ client }: { client: ApiClient }) {
  const { snapshot, store } = useConnectionsStore(client);
  return (
    <div className="connection-sidebar-collection">
      <ConnectionCollection compact snapshot={snapshot} store={store} />
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
