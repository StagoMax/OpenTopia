import { memo } from "react";
import { useApplicationLanguage } from "../../ApplicationLanguageProvider";
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
  const { language, t } = useApplicationLanguage();
  if (!selectedConnection) {
    return (
      <section
        className="agent-connection-grants__operations"
        aria-label={t("flow.connectionGrants.operationAria")}
      >
        <div className="agent-connection-grants__empty-state">
          <Wrench aria-hidden="true" size={18} />
          <strong>{t("flow.connectionGrants.selectConnection")}</strong>
          <span>{t("flow.connectionGrants.selectConnectionHint")}</span>
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
      aria-label={t("flow.connectionGrants.operationAria")}
    >
      <header className="agent-connection-grants__operations-header">
        <span>
          <strong>{selectedConnection.name}</strong>
          <small>
            {t("flow.connectionGrants.authentication")}
            {authVerificationLabel(
              selectedConnection.authContext.verification,
              language,
            )}
            {shownRevision
              ? ` · ${t("flow.connectionGrants.capabilityRevision")} r${shownRevision.revision}`
              : ""}
          </small>
        </span>
        {selectedBinding ? (
          <Button
            aria-label={`${t("flow.connectionGrants.removeAllAria")} ${selectedConnection.name}`}
            onClick={onRemove}
            size="compact"
            variant="quiet"
          >
            <Trash2 aria-hidden="true" size={14} />{" "}
            {t("flow.connectionGrants.remove")}
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
          {t("flow.connectionGrants.loadingRevisions")}
        </div>
      ) : null}
      {revisionsState?.status === "error" ? (
        <div className="agent-connection-grants__state is-error" role="alert">
          <CircleAlert aria-hidden="true" size={16} />
          <span>{revisionsState.error}</span>
          <Button onClick={onRetryRevisions} size="compact" variant="quiet">
            {t("flow.connectionGrants.retry")}
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
            <strong>{t("flow.connectionGrants.staleTitle")}</strong>
            <small>
              {freshness.changedOperationIds.length}{" "}
              {t("flow.connectionGrants.changedItems")}
              {language === "zh-CN" ? "，" : ", "}
              {freshness.removedOperationIds.length}{" "}
              {t("flow.connectionGrants.removedItems")}
            </small>
          </span>
          {activeRevision ? (
            <Button onClick={onRebase} size="compact" variant="secondary">
              {t("flow.connectionGrants.rebase")} r{activeRevision.revision}
            </Button>
          ) : null}
        </div>
      ) : null}
      {freshness?.state === "current" && pinnedToOlderRevision ? (
        <div className="agent-connection-grants__notice" role="note">
          <RefreshCw aria-hidden="true" size={16} />
          <span>
            <strong>{t("flow.connectionGrants.newRevisionTitle")}</strong>
            <small>{t("flow.connectionGrants.newRevisionDetail")}</small>
          </span>
          <Button onClick={onRebase} size="compact" variant="secondary">
            {t("flow.connectionGrants.viewRevision")} r
            {activeRevision?.revision}
          </Button>
        </div>
      ) : null}
      {freshness?.state === "unavailable" ? (
        <div className="agent-connection-grants__notice is-error" role="alert">
          <CircleAlert aria-hidden="true" size={16} />
          <span>{t("flow.connectionGrants.unavailableRevision")}</span>
        </div>
      ) : null}

      {shownRevision ? (
        <>
          {shownRevision.capabilities.length > 8 ? (
            <TextField
              label={
                <span className="agent-connection-grants__search-label">
                  <Search aria-hidden="true" size={14} />{" "}
                  {t("flow.connectionGrants.searchOperations")}
                </span>
              }
              placeholder={t(
                "flow.connectionGrants.searchOperationsPlaceholder",
              )}
              value={operationQuery}
              onChange={(event) => onOperationQueryChange(event.target.value)}
            />
          ) : null}
          <div className="agent-connection-grants__operation-summary">
            <span>
              <ShieldCheck aria-hidden="true" size={14} />
              {t("flow.connectionGrants.authorizedCount")}{" "}
              {selectedOperationIds.size} / {shownRevision.capabilities.length}
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
              {t("flow.connectionGrants.noOperationMatch")}
            </p>
          ) : null}
          {visibleOperations < filteredOperationCount ? (
            <Button onClick={onShowMore} variant="quiet">
              {t("flow.connectionGrants.moreOperations")}
              {filteredOperationCount - visibleOperations}
              {language === "zh-CN" ? "）" : ")"}
            </Button>
          ) : null}
        </>
      ) : revisionsState?.status === "ready" ? (
        <p className="agent-connection-grants__empty">
          {t("flow.connectionGrants.noRevisions")}
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
  const { t } = useApplicationLanguage();
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
        {stale ? (
          <Badge variant="warning">
            {t("flow.connectionGrants.descriptionChanged")}
          </Badge>
        ) : null}
        {operation.permissionLabels.map((label) => (
          <Badge key={label}>{label}</Badge>
        ))}
      </span>
    </label>
  );
});
