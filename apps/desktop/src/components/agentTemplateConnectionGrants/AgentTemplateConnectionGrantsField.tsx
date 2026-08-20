import {
  useCallback,
  useDeferredValue,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { CircleAlert, Trash2 } from "lucide-react";
import type { ApiClient } from "../../api/client";
import type {
  AgentConnectionBinding,
  Connection,
  ConnectionCapabilityRevision,
  IntegrationDefinition,
} from "../../types";
import { Button } from "../ui";
import {
  connectionAccountLabel,
  definitionForConnection,
} from "../connections/model";
import { ConnectionGrantCatalog } from "./ConnectionGrantCatalog";
import {
  OperationGrantEditor,
  type RevisionsState,
} from "./OperationGrantEditor";
import {
  bindingFreshness,
  connectionGrantEligibility,
  filterCapabilities,
  hasLegacyMcpProjection,
  rebaseBinding,
  removeConnectionBinding,
  replaceConnectionBinding,
  toggleOperationGrant,
} from "./model";
import "../../styles/agent-template-connection-grants.css";

const OPERATION_PAGE_SIZE = 40;

type CatalogState = {
  status: "loading" | "ready" | "error";
  connections: Connection[];
  definitions: IntegrationDefinition[];
  error: string | null;
};

export type AgentTemplateConnectionGrantsFieldProps = {
  client: ApiClient | null;
  disabled?: boolean;
  legacyAllowAllMcpServers: boolean;
  legacyMcpServerIds: readonly string[];
  value: readonly AgentConnectionBinding[];
  onChange(value: AgentConnectionBinding[]): void;
  onClearLegacyMcpServers(): void;
};

export function AgentTemplateConnectionGrantsField({
  client,
  disabled = false,
  legacyAllowAllMcpServers,
  legacyMcpServerIds,
  value,
  onChange,
  onClearLegacyMcpServers,
}: AgentTemplateConnectionGrantsFieldProps) {
  const [catalog, setCatalog] = useState<CatalogState>({
    status: "loading",
    connections: [],
    definitions: [],
    error: null,
  });
  const [selectedConnectionId, setSelectedConnectionId] = useState<
    string | null
  >(value[0]?.connectionId ?? null);
  const [revisionsByConnection, setRevisionsByConnection] = useState<
    Record<string, RevisionsState>
  >({});
  const revisionsCache = useRef<Record<string, RevisionsState>>({});
  const revisionsGeneration = useRef(0);
  const [revisionsRetryNonce, setRevisionsRetryNonce] = useState(0);
  const [connectionQuery, setConnectionQuery] = useState("");
  const [operationQuery, setOperationQuery] = useState("");
  const [visibleOperations, setVisibleOperations] =
    useState(OPERATION_PAGE_SIZE);
  const deferredConnectionQuery = useDeferredValue(connectionQuery);
  const deferredOperationQuery = useDeferredValue(operationQuery);

  useEffect(() => {
    revisionsGeneration.current += 1;
    revisionsCache.current = {};
    setRevisionsByConnection({});
    if (!client) {
      setCatalog({
        status: "error",
        connections: [],
        definitions: [],
        error: "后端未连接，无法读取 Connections",
      });
      return;
    }
    const controller = new AbortController();
    setCatalog((current) => ({ ...current, status: "loading", error: null }));
    void Promise.all([
      client.listConnections({}, controller.signal),
      client.listIntegrationDefinitions(controller.signal),
    ])
      .then(([connections, definitions]) => {
        if (controller.signal.aborted) return;
        setCatalog({
          status: "ready",
          connections: [...connections].sort((left, right) =>
            left.name.localeCompare(right.name),
          ),
          definitions,
          error: null,
        });
        setSelectedConnectionId((current) =>
          current && connections.some((item) => item.id === current)
            ? current
            : (value[0]?.connectionId ?? connections[0]?.id ?? null),
        );
      })
      .catch((error: unknown) => {
        if (controller.signal.aborted) return;
        setCatalog({
          status: "error",
          connections: [],
          definitions: [],
          error: errorMessage(error),
        });
      });
    return () => controller.abort();
  }, [client]);

  useEffect(() => {
    if (!client || !selectedConnectionId) return;
    if (revisionsCache.current[selectedConnectionId]) return;
    const generation = revisionsGeneration.current;
    const loadingState: RevisionsState = {
      status: "loading",
      revisions: [],
      error: null,
    };
    revisionsCache.current = {
      ...revisionsCache.current,
      [selectedConnectionId]: loadingState,
    };
    setRevisionsByConnection(revisionsCache.current);
    void client
      .listConnectionCapabilityRevisions(selectedConnectionId)
      .then((revisions) => {
        if (generation !== revisionsGeneration.current) return;
        revisionsCache.current = {
          ...revisionsCache.current,
          [selectedConnectionId]: {
            status: "ready",
            revisions: [...revisions].sort(
              (left, right) => right.revision - left.revision,
            ),
            error: null,
          },
        };
        setRevisionsByConnection(revisionsCache.current);
      })
      .catch((error: unknown) => {
        if (generation !== revisionsGeneration.current) return;
        revisionsCache.current = {
          ...revisionsCache.current,
          [selectedConnectionId]: {
            status: "error",
            revisions: [],
            error: errorMessage(error),
          },
        };
        setRevisionsByConnection(revisionsCache.current);
      });
  }, [client, revisionsRetryNonce, selectedConnectionId]);

  useEffect(() => {
    setOperationQuery("");
    setVisibleOperations(OPERATION_PAGE_SIZE);
  }, [selectedConnectionId]);

  const filteredConnections = useMemo(() => {
    const query = deferredConnectionQuery.trim().toLocaleLowerCase();
    if (!query) return catalog.connections;
    return catalog.connections.filter((connection) => {
      const definition = definitionForConnection(
        catalog.definitions,
        connection,
      );
      return [
        connection.name,
        connection.environment,
        connectionAccountLabel(connection),
        definition?.name ?? "",
      ].some((part) => part.toLocaleLowerCase().includes(query));
    });
  }, [catalog.connections, catalog.definitions, deferredConnectionQuery]);

  const selectedConnection = catalog.connections.find(
    (connection) => connection.id === selectedConnectionId,
  );
  const selectedDefinition = selectedConnection
    ? (definitionForConnection(catalog.definitions, selectedConnection) ?? null)
    : null;
  const selectedBinding = value.find(
    (binding) => binding.connectionId === selectedConnectionId,
  );
  const revisionsState = selectedConnectionId
    ? revisionsByConnection[selectedConnectionId]
    : undefined;
  const activeRevision = selectedConnection
    ? revisionsState?.revisions.find(
        (revision) =>
          revision.revision === selectedConnection.activeCapabilityRevision,
      )
    : undefined;
  const pinnedRevision = selectedBinding
    ? revisionsState?.revisions.find(
        (revision) => revision.revision === selectedBinding.capabilityRevision,
      )
    : activeRevision;
  const shownRevision = selectedBinding ? pinnedRevision : activeRevision;
  const freshness = selectedBinding
    ? bindingFreshness(selectedBinding, pinnedRevision, activeRevision)
    : null;
  const eligibility = selectedConnection
    ? connectionGrantEligibility(selectedConnection, selectedDefinition)
    : null;
  const filteredOperations = useMemo(
    () =>
      filterCapabilities(
        shownRevision?.capabilities ?? [],
        deferredOperationQuery,
      ),
    [deferredOperationQuery, shownRevision],
  );
  const renderedOperations = filteredOperations.slice(0, visibleOperations);
  const selectedOperationIds = useMemo(
    () =>
      new Set(
        selectedBinding?.operationGrants.map((grant) => grant.operationId) ??
          [],
      ),
    [selectedBinding],
  );
  const structuredConnectionIds = useMemo(
    () => new Set(value.map((binding) => binding.connectionId)),
    [value],
  );
  const legacyOnlyIds = legacyMcpServerIds;
  const hasLegacyProjection = hasLegacyMcpProjection(
    legacyAllowAllMcpServers,
    legacyOnlyIds,
  );
  const missingStructuredBindings = value.filter(
    (binding) =>
      !catalog.connections.some(
        (connection) => connection.id === binding.connectionId,
      ),
  );

  function retryCatalog() {
    if (!client) return;
    setCatalog((current) => ({ ...current, status: "loading", error: null }));
    void Promise.all([
      client.listConnections(),
      client.listIntegrationDefinitions(),
    ])
      .then(([connections, definitions]) =>
        setCatalog({
          status: "ready",
          connections: [...connections].sort((left, right) =>
            left.name.localeCompare(right.name),
          ),
          definitions,
          error: null,
        }),
      )
      .catch((error: unknown) =>
        setCatalog((current) => ({
          ...current,
          status: "error",
          error: errorMessage(error),
        })),
      );
  }

  function retryRevisions(connectionId: string) {
    const nextCache = { ...revisionsCache.current };
    delete nextCache[connectionId];
    revisionsCache.current = nextCache;
    setRevisionsByConnection((current) => {
      const next = { ...current };
      delete next[connectionId];
      return next;
    });
    setRevisionsRetryNonce((current) => current + 1);
  }

  const handleToggleOperation = useCallback(
    (operationId: string) => {
      if (
        disabled ||
        !selectedConnection ||
        !activeRevision ||
        !eligibility?.selectable ||
        hasLegacyProjection
      ) {
        return;
      }
      const binding = selectedBinding ?? {
        connectionId: selectedConnection.id,
        capabilityRevision: activeRevision.revision,
        operationGrants: [],
      };
      const nextBinding = toggleOperationGrant(binding, operationId);
      onChange(
        nextBinding.operationGrants.length > 0
          ? replaceConnectionBinding(value, nextBinding)
          : removeConnectionBinding(value, selectedConnection.id),
      );
    },
    [
      activeRevision,
      disabled,
      eligibility?.selectable,
      hasLegacyProjection,
      onChange,
      selectedBinding,
      selectedConnection,
      value,
    ],
  );

  function rebaseSelectedBinding() {
    if (!selectedBinding || !activeRevision || !selectedConnection) return;
    const rebased = rebaseBinding(selectedBinding, activeRevision);
    onChange(
      rebased.operationGrants.length > 0
        ? replaceConnectionBinding(value, rebased)
        : removeConnectionBinding(value, selectedConnection.id),
    );
  }

  return (
    <fieldset className="agent-connection-grants" disabled={disabled}>
      <legend>Connections 与操作权限</legend>
      <p className="agent-connection-grants__hint">
        权限固定到不可变能力修订；新增工具不会自动扩权，已授权工具变更或移除会标记为失效。
      </p>

      {hasLegacyProjection ? (
        <div className="agent-connection-grants__legacy" role="note">
          <CircleAlert aria-hidden="true" size={16} />
          <span>
            <strong>Legacy MCP 绑定（只读）</strong>
            <small>
              旧模板没有操作级快照。保留兼容执行，但需新建版本后重新选择操作。
            </small>
            <code>
              {legacyAllowAllMcpServers
                ? "allowAllMcpServers = true"
                : legacyOnlyIds.join(", ")}
            </code>
          </span>
          <Button
            disabled={disabled}
            onClick={onClearLegacyMcpServers}
            size="compact"
            variant="secondary"
          >
            开始迁移
          </Button>
          <small className="agent-connection-grants__legacy-action-hint">
            此操作会从当前草稿移除 Legacy MCP 绑定；随后必须重新选择所需操作。
          </small>
        </div>
      ) : null}

      {catalog.status === "ready" && missingStructuredBindings.length > 0 ? (
        <div className="agent-connection-grants__missing" role="alert">
          <CircleAlert aria-hidden="true" size={16} />
          <span>
            <strong>Connection 不可用</strong>
            <small>
              下列结构化授权找不到对应 Connection，将在发布与运行时 fail
              closed。
            </small>
          </span>
          <div>
            {missingStructuredBindings.map((binding) => (
              <span key={binding.connectionId}>
                <code>{binding.connectionId}</code>
                <Button
                  aria-label={`移除不可用 Connection ${binding.connectionId}`}
                  onClick={() =>
                    onChange(
                      removeConnectionBinding(value, binding.connectionId),
                    )
                  }
                  size="compact"
                  variant="quiet"
                >
                  <Trash2 aria-hidden="true" size={14} /> 移除
                </Button>
              </span>
            ))}
          </div>
        </div>
      ) : null}

      {catalog.status === "loading" ? (
        <div className="agent-connection-grants__state" role="status">
          正在加载 Connections…
        </div>
      ) : null}
      {catalog.status === "error" ? (
        <div className="agent-connection-grants__state is-error" role="alert">
          <CircleAlert aria-hidden="true" size={16} />
          <span>{catalog.error}</span>
          <Button size="compact" variant="quiet" onClick={retryCatalog}>
            重试
          </Button>
        </div>
      ) : null}

      {catalog.status === "ready" ? (
        <div className="agent-connection-grants__layout">
          <ConnectionGrantCatalog
            allConnections={catalog.connections}
            definitions={catalog.definitions}
            filteredConnections={filteredConnections}
            onQueryChange={setConnectionQuery}
            onSelect={setSelectedConnectionId}
            query={connectionQuery}
            selectedConnectionId={selectedConnectionId}
            structuredConnectionIds={structuredConnectionIds}
          />
          <OperationGrantEditor
            activeRevision={activeRevision}
            disabled={disabled}
            eligibility={eligibility}
            filteredOperationCount={filteredOperations.length}
            freshness={freshness}
            hasLegacyProjection={hasLegacyProjection}
            onOperationQueryChange={setOperationQuery}
            onRebase={rebaseSelectedBinding}
            onRemove={() => {
              if (!selectedConnection) return;
              onChange(removeConnectionBinding(value, selectedConnection.id));
            }}
            onRetryRevisions={() => {
              if (selectedConnection) retryRevisions(selectedConnection.id);
            }}
            onShowMore={() =>
              setVisibleOperations((current) => current + OPERATION_PAGE_SIZE)
            }
            onToggleOperation={handleToggleOperation}
            operationQuery={operationQuery}
            renderedOperations={renderedOperations}
            revisionsState={revisionsState}
            selectedBinding={selectedBinding}
            selectedConnection={selectedConnection}
            selectedOperationIds={selectedOperationIds}
            shownRevision={shownRevision}
            visibleOperations={visibleOperations}
          />
        </div>
      ) : null}
    </fieldset>
  );
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
