import {
  Cable,
  CircleAlert,
  RefreshCw,
  ShieldCheck,
  Wrench,
} from "lucide-react";
import type { AgentTemplateConnectionAccessView } from "../../api/generated/desktop-http-v1.generated";
import { useApplicationLanguage } from "../../ApplicationLanguageProvider";
import { Badge, Button } from "../ui";

const MAX_EFFECTIVE_TOOL_NAMES = 12;

export function AgentTemplateConnectionAccessSummary({
  access,
  error,
  loading,
  onRetry,
}: {
  access: AgentTemplateConnectionAccessView | null;
  error: string | null;
  loading: boolean;
  onRetry(): void;
}) {
  const { t } = useApplicationLanguage();
  return (
    <section className="agent-template-panel__connection-access">
      <header className="agent-template-panel__section-title">
        <ShieldCheck aria-hidden="true" size={14} />
        {t("flow.connectionAccess.boundary")}
        {access ? (
          <Badge variant={access.valid ? "success" : "danger"}>
            {access.valid
              ? t("flow.connectionAccess.valid")
              : t("flow.connectionAccess.blocked")}
          </Badge>
        ) : null}
        {access?.mode === "legacy" ? (
          <Badge variant="warning">{t("flow.connectionAccess.legacy")}</Badge>
        ) : null}
      </header>

      {loading ? (
        <p className="agent-template-panel__empty" role="status">
          {t("flow.connectionAccess.loading")}
        </p>
      ) : null}
      {error ? (
        <div
          className="agent-template-panel__connection-access-state is-error"
          role="alert"
        >
          <CircleAlert aria-hidden="true" size={16} />
          <span>{error}</span>
          <Button onClick={onRetry} size="compact" variant="quiet">
            <RefreshCw aria-hidden="true" size={14} />{" "}
            {t("flow.connectionGrants.retry")}
          </Button>
        </div>
      ) : null}

      {access?.mode === "none" ? (
        <p className="agent-template-panel__empty">
          {t("flow.connectionAccess.none")}
        </p>
      ) : null}
      {access?.mode === "legacy" ? (
        <div
          className="agent-template-panel__connection-access-state is-warning"
          role="note"
        >
          <CircleAlert aria-hidden="true" size={16} />
          <span>{t("flow.connectionAccess.legacyDetail")}</span>
        </div>
      ) : null}

      {access?.bindings.length ? (
        <div className="agent-template-panel__connection-bindings">
          {access.bindings.map((binding) => (
            <article key={binding.connectionId}>
              <header>
                <Cable aria-hidden="true" size={14} />
                <span>
                  <strong>
                    {binding.connectionName ?? binding.connectionId}
                  </strong>
                  <small>
                    r{binding.capabilityRevision} · {binding.operations.length}{" "}
                    {t("flow.connectionAccess.operations")}
                  </small>
                </span>
                <Badge variant={binding.valid ? "success" : "danger"}>
                  {binding.valid
                    ? t("flow.connectionAccess.executable")
                    : t("flow.connectionAccess.failClosed")}
                </Badge>
              </header>
              {binding.operations.length ? (
                <ul className="agent-template-panel__connection-operations">
                  {binding.operations.map((operation) => (
                    <li key={operation.operationId}>
                      <Wrench aria-hidden="true" size={14} />
                      <span>
                        <strong>
                          {operation.displayName ??
                            operation.name ??
                            operation.operationId}
                        </strong>
                        <code>{operation.operationId}</code>
                        {operation.modelToolName ? (
                          <small>
                            {t("flow.connectionAccess.modelTool")}
                            {operation.modelToolName}
                          </small>
                        ) : null}
                      </span>
                      <span>
                        {operation.permissionLabels.map((label) => (
                          <Badge key={label}>{label}</Badge>
                        ))}
                      </span>
                    </li>
                  ))}
                </ul>
              ) : null}
              {binding.issues.length ? (
                <IssueList issues={binding.issues} />
              ) : null}
            </article>
          ))}
        </div>
      ) : null}

      {access?.issues.length && access.bindings.length === 0 ? (
        <IssueList issues={access.issues} />
      ) : null}

      {access?.effectiveModelToolNames.length ? (
        <div className="agent-template-panel__effective-tools">
          <strong>{t("flow.connectionAccess.effectiveTools")}</strong>
          <span>
            {access.effectiveModelToolNames
              .slice(0, MAX_EFFECTIVE_TOOL_NAMES)
              .map((toolName) => (
                <code key={toolName}>{toolName}</code>
              ))}
            {access.effectiveModelToolNames.length >
            MAX_EFFECTIVE_TOOL_NAMES ? (
              <small>
                +
                {access.effectiveModelToolNames.length -
                  MAX_EFFECTIVE_TOOL_NAMES}{" "}
                {t("flow.connectionAccess.items")}
              </small>
            ) : null}
          </span>
        </div>
      ) : null}
    </section>
  );
}

function IssueList({
  issues,
}: {
  issues: AgentTemplateConnectionAccessView["issues"];
}) {
  const { t } = useApplicationLanguage();
  return (
    <ul className="agent-template-panel__connection-issues">
      {issues.map((issue, index) => (
        <li
          className={issue.severity === "error" ? "is-error" : "is-warning"}
          key={`${issue.code}:${issue.connectionId ?? ""}:${issue.operationId ?? ""}:${index}`}
        >
          <CircleAlert aria-hidden="true" size={14} />
          <span>{issueMessage(issue.code, issue.message, t)}</span>
          <code>{issue.code}</code>
        </li>
      ))}
    </ul>
  );
}

function issueMessage(
  code: string,
  fallback: string,
  t: ReturnType<typeof useApplicationLanguage>["t"],
): string {
  return (
    {
      legacy_mcp_server_grants: t("flow.connectionAccess.issue.legacyGrants"),
      connection_not_found: t("flow.connectionAccess.issue.connectionNotFound"),
      integration_definition_not_found: t(
        "flow.connectionAccess.issue.definitionNotFound",
      ),
      integration_definition_disabled: t(
        "flow.connectionAccess.issue.definitionDisabled",
      ),
      connection_disabled: t("flow.connectionAccess.issue.connectionDisabled"),
      connection_not_ready: t("flow.connectionAccess.issue.connectionNotReady"),
      legacy_auth_unverified: t("flow.connectionAccess.issue.legacyAuth"),
      connection_auth_unverified: t(
        "flow.connectionAccess.issue.authUnverified",
      ),
      capability_revision_not_found: t(
        "flow.connectionAccess.issue.revisionNotFound",
      ),
      active_capability_revision_not_found: t(
        "flow.connectionAccess.issue.activeRevisionNotFound",
      ),
      operation_not_in_pinned_revision: t(
        "flow.connectionAccess.issue.operationNotPinned",
      ),
      operation_removed: t("flow.connectionAccess.issue.operationRemoved"),
      operation_descriptor_changed: t(
        "flow.connectionAccess.issue.operationChanged",
      ),
      operation_runtime_mismatch: t(
        "flow.connectionAccess.issue.runtimeMismatch",
      ),
      mcp_runtime_not_found: t("flow.connectionAccess.issue.runtimeNotFound"),
      mcp_runtime_disabled: t("flow.connectionAccess.issue.runtimeDisabled"),
    }[code] ?? fallback
  );
}
