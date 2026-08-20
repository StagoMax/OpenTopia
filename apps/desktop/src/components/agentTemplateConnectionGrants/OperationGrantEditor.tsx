import { memo } from "react";
import {
  CircleAlert,
  RefreshCw,
  Search,
  ShieldCheck,
  Trash2,
  Wrench,
} from "lucide-react";
import type {
  AgentConnectionBinding,
  Connection,
  ConnectionCapability,
  ConnectionCapabilityRevision,
} from "../../types";
import { Badge, Button, TextField } from "../ui";
import { authVerificationLabel } from "../connections/model";
import type {
  ConnectionBindingFreshness,
  ConnectionGrantEligibility,
} from "./model";

export type RevisionsState = {
  status: "loading" | "ready" | "error";
  revisions: ConnectionCapabilityRevision[];
  error: string | null;
};

export function OperationGrantEditor({
  activeRevision,
  disabled,
  eligibility,
  filteredOperationCount,
  freshness,
  hasLegacyProjection,
  onOperationQueryChange,
  onRebase,
  onRemove,
  onRetryRevisions,
  onShowMore,
  onToggleOperation,
  operationQuery,
  renderedOperations,
  revisionsState,
  selectedBinding,
  selectedConnection,
  selectedOperationIds,
  shownRevision,
  visibleOperations,
}: {
  activeRevision?: ConnectionCapabilityRevision;
  disabled: boolean;
  eligibility: ConnectionGrantEligibility | null;
  filteredOperationCount: number;
  freshness: ConnectionBindingFreshness | null;
  hasLegacyProjection: boolean;
  onOperationQueryChange(value: string): void;
  onRebase(): void;
  onRemove(): void;
  onRetryRevisions(): void;
  onShowMore(): void;
  onToggleOperation(operationId: string): void;
  operationQuery: string;
  renderedOperations: readonly ConnectionCapability[];
  revisionsState?: RevisionsState;
  selectedBinding?: AgentConnectionBinding;
  selectedConnection?: Connection;
  selectedOperationIds: ReadonlySet<string>;
  shownRevision?: ConnectionCapabilityRevision;
  visibleOperations: number;
}) {
  if (!selectedConnection) {
    return (
      <section
        className="agent-connection-grants__operations"
        aria-label="Connection 操作权限"
      >
        <div className="agent-connection-grants__empty-state">
          <Wrench aria-hidden="true" size={18} />
          <strong>选择一个 Connection</strong>
          <span>查看其固定能力修订并授予具体操作。</span>
        </div>
      </section>
    );
  }

  const pinnedToOlderRevision = Boolean(
    selectedBinding &&
    activeRevision &&
    selectedBinding.capabilityRevision !== activeRevision.revision,
  );

  return (
    <section
      className="agent-connection-grants__operations"
      aria-label="Connection 操作权限"
    >
      <header className="agent-connection-grants__operations-header">
        <span>
          <strong>{selectedConnection.name}</strong>
          <small>
            认证：
            {authVerificationLabel(selectedConnection.authContext.verification)}
            {shownRevision ? ` · 能力修订 r${shownRevision.revision}` : ""}
          </small>
        </span>
        {selectedBinding ? (
          <Button
            aria-label={`移除 ${selectedConnection.name} 的全部操作权限`}
            onClick={onRemove}
            size="compact"
            variant="quiet"
          >
            <Trash2 aria-hidden="true" size={14} /> 移除
          </Button>
        ) : null}
      </header>

      {eligibility?.reason ? (
        <div
          className={`agent-connection-grants__notice${eligibility.warning ? " is-warning" : " is-error"}`}
          role={eligibility.warning ? "note" : "alert"}
        >
          <CircleAlert aria-hidden="true" size={16} />
          <span>{eligibility.reason}</span>
        </div>
      ) : null}

      {revisionsState?.status === "loading" || !revisionsState ? (
        <div className="agent-connection-grants__state" role="status">
          正在读取能力修订…
        </div>
      ) : null}
      {revisionsState?.status === "error" ? (
        <div className="agent-connection-grants__state is-error" role="alert">
          <CircleAlert aria-hidden="true" size={16} />
          <span>{revisionsState.error}</span>
          <Button onClick={onRetryRevisions} size="compact" variant="quiet">
            重试
          </Button>
        </div>
      ) : null}

      {freshness?.state === "stale" ? (
        <div
          className="agent-connection-grants__notice is-warning"
          role="alert"
        >
          <RefreshCw aria-hidden="true" size={16} />
          <span>
            <strong>授权快照已过期</strong>
            <small>
              {freshness.changedOperationIds.length} 项描述已变更，
              {freshness.removedOperationIds.length}{" "}
              项已移除。新增操作未自动授权。
            </small>
          </span>
          {activeRevision ? (
            <Button onClick={onRebase} size="compact" variant="secondary">
              更新并重新审阅 r{activeRevision.revision}
            </Button>
          ) : null}
        </div>
      ) : null}
      {freshness?.state === "current" && pinnedToOlderRevision ? (
        <div className="agent-connection-grants__notice" role="note">
          <RefreshCw aria-hidden="true" size={16} />
          <span>
            <strong>发现新的能力修订</strong>
            <small>
              当前授权仍然有效；新增操作不会自动授权。更新后可查看并选择新操作。
            </small>
          </span>
          <Button onClick={onRebase} size="compact" variant="secondary">
            查看 r{activeRevision?.revision}
          </Button>
        </div>
      ) : null}
      {freshness?.state === "unavailable" ? (
        <div className="agent-connection-grants__notice is-error" role="alert">
          <CircleAlert aria-hidden="true" size={16} />
          <span>固定的能力修订不可用；该授权将 fail closed。</span>
        </div>
      ) : null}

      {shownRevision ? (
        <>
          {shownRevision.capabilities.length > 8 ? (
            <TextField
              label={
                <span className="agent-connection-grants__search-label">
                  <Search aria-hidden="true" size={14} /> 搜索操作
                </span>
              }
              placeholder="名称、说明、权限标签或 operation ID"
              value={operationQuery}
              onChange={(event) => onOperationQueryChange(event.target.value)}
            />
          ) : null}
          <div className="agent-connection-grants__operation-summary">
            <span>
              <ShieldCheck aria-hidden="true" size={14} />
              已授权 {selectedOperationIds.size} /{" "}
              {shownRevision.capabilities.length}
            </span>
            <code>{shownRevision.contentHash.slice(0, 12)}</code>
          </div>
          <div className="agent-connection-grants__operation-list">
            {renderedOperations.map((operation) => (
              <OperationGrantRow
                checked={selectedOperationIds.has(operation.capabilityId)}
                disabled={
                  disabled ||
                  !eligibility?.selectable ||
                  hasLegacyProjection ||
                  pinnedToOlderRevision
                }
                key={operation.capabilityId}
                operation={operation}
                stale={
                  freshness?.changedOperationIds.includes(
                    operation.capabilityId,
                  ) ?? false
                }
                onToggle={onToggleOperation}
              />
            ))}
          </div>
          {filteredOperationCount === 0 ? (
            <p className="agent-connection-grants__empty">
              没有匹配的操作，请调整搜索条件。
            </p>
          ) : null}
          {visibleOperations < filteredOperationCount ? (
            <Button onClick={onShowMore} variant="quiet">
              显示更多（剩余{filteredOperationCount - visibleOperations}）
            </Button>
          ) : null}
        </>
      ) : revisionsState?.status === "ready" ? (
        <p className="agent-connection-grants__empty">
          没有可用能力修订，请先在 Connections 中刷新能力。
        </p>
      ) : null}
    </section>
  );
}

const OperationGrantRow = memo(function OperationGrantRow({
  checked,
  disabled,
  operation,
  stale,
  onToggle,
}: {
  checked: boolean;
  disabled: boolean;
  operation: ConnectionCapability;
  stale: boolean;
  onToggle(operationId: string): void;
}) {
  return (
    <label
      className={`agent-connection-grants__operation${stale ? " is-stale" : ""}`}
    >
      <input
        checked={checked}
        disabled={disabled}
        onChange={() => onToggle(operation.capabilityId)}
        type="checkbox"
      />
      <Wrench aria-hidden="true" size={14} />
      <span>
        <strong>{operation.displayName || operation.name}</strong>
        <code>{operation.capabilityId}</code>
        {operation.description ? <small>{operation.description}</small> : null}
      </span>
      <span className="agent-connection-grants__permission-labels">
        {stale ? <Badge variant="warning">描述已变更</Badge> : null}
        {operation.permissionLabels.map((label) => (
          <Badge key={label}>{label}</Badge>
        ))}
      </span>
    </label>
  );
});
