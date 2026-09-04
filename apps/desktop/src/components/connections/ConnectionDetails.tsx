import {
  AlertTriangle,
  Cable,
  CheckCircle2,
  Clock3,
  Info,
  Pencil,
  RotateCw,
  Server,
  ShieldCheck,
  Wrench,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useApplicationLanguage } from "../../ApplicationLanguageProvider";
import type { ApplicationLanguage } from "../../applicationLanguage";
import type {
  Connection,
  ConnectionCapabilityRevision,
  IntegrationDefinition,
} from "../../types";
import { Badge, Button, Select } from "../ui";
import {
  authSchemeLabel,
  authVerificationLabel,
  connectionAccountLabel,
  connectionProblemAreaLabel,
  connectionProblems,
  connectionStatusLabel,
  connectionStatusVariant,
  integrationKindLabel,
  ownerTypeLabel,
} from "./model";
import type {
  ConnectionNotice,
  ConnectionsSnapshot,
  ConnectionsStore,
} from "./store";

export type ConnectionDetailsProps = {
  connection: Connection;
  definition?: IntegrationDefinition;
  snapshot: ConnectionsSnapshot;
  store: ConnectionsStore;
  usage?: {
    agentNames: readonly string[];
    flowNames: readonly string[];
  };
  variant?: "full" | "core" | "inspector";
};

export function ConnectionDetails({
  connection,
  definition,
  snapshot,
  store,
  usage,
  variant = "full",
}: ConnectionDetailsProps) {
  const { language, t } = useApplicationLanguage();
  const revisions = snapshot.capabilityRevisions[connection.id];
  const defaultRevision = useMemo(
    () =>
      revisions?.find(
        (revision) => revision.revision === connection.activeCapabilityRevision,
      ) ?? revisions?.[0],
    [connection.activeCapabilityRevision, revisions],
  );
  const [selectedRevision, setSelectedRevision] = useState("");
  useEffect(() => {
    setSelectedRevision(
      defaultRevision ? String(defaultRevision.revision) : "",
    );
  }, [connection.id, defaultRevision]);
  const revision =
    revisions?.find((item) => String(item.revision) === selectedRevision) ??
    defaultRevision;
  const server = snapshot.mcpServers.find(
    ({ server: item }) => item.serverId === connection.runtimeBinding.serverId,
  );
  const testing = snapshot.busyAction === `test:${connection.id}`;
  const busy = Boolean(snapshot.busyAction);
  const health =
    snapshot.lastHealth?.connectionId === connection.id
      ? snapshot.lastHealth.health
      : null;
  const problems = connectionProblems(connection, language);
  const usageSummary = describeUsage(usage, t);

  return (
    <article className={`connection-details connection-details--${variant}`}>
      {variant === "full" ? (
        <header className="connection-details__header">
          <span className="connections-icon connections-icon--large">
            <Cable aria-hidden="true" size={18} />
          </span>
          <span className="connection-details__title">
            <span>
              <h2>{connection.name}</h2>
              <Badge variant={connectionStatusVariant(connection.status)}>
                {connectionStatusLabel(connection.status, language)}
              </Badge>
            </span>
            <p>
              {definition?.name ?? t("flow.connection.details.unknownService")}{" "}
              · {connectionAccountLabel(connection, language)}
            </p>
          </span>
        </header>
      ) : null}

      {variant !== "inspector" ? (
        <>
          <section
            className={`connection-operation-summary ${problems.length ? "is-warning" : "is-healthy"}`}
          >
            <span aria-hidden="true">
              {problems.length ? (
                <AlertTriangle size={20} />
              ) : (
                <CheckCircle2 size={20} />
              )}
            </span>
            <div>
              <small>
                {definition?.name ??
                  t("flow.connection.details.unknownService")}{" "}
                · {connectionAccountLabel(connection, language)}
              </small>
              <span>
                <h2>{connection.name}</h2>
                <Badge variant={connectionStatusVariant(connection.status)}>
                  {connectionStatusLabel(connection.status, language)}
                </Badge>
              </span>
              <p>
                {problems[0]?.detail ??
                  `${usageSummary}${t("flow.connection.details.availableSuffix")}`}
              </p>
            </div>
          </section>

          <dl
            className="connection-operation-facts"
            aria-label={t("flow.connection.details.summaryAria")}
          >
            <div>
              <dt>{t("flow.connection.details.usageScope")}</dt>
              <dd>{usageSummary}</dd>
            </div>
            <div>
              <dt>{t("flow.connection.details.lastCheck")}</dt>
              <dd>{formatDate(connection.lastTestedAt, language)}</dd>
            </div>
            <div>
              <dt>{t("flow.connection.details.availableCapabilities")}</dt>
              <dd>
                {revision
                  ? `${revision.capabilities.length} ${t("flow.connection.details.items")}`
                  : t("flow.connection.details.notDiscovered")}
              </dd>
            </div>
          </dl>

          {problems.length > 0 ? (
            <section
              className="connection-attention"
              aria-label={t("flow.connection.details.attentionAria")}
            >
              <header>
                <strong>{t("flow.connection.details.attention")}</strong>
                <span className="connection-attention__actions">
                  <Button
                    disabled={busy}
                    onClick={() => store.beginEdit()}
                    size="compact"
                    variant="quiet"
                  >
                    <Pencil aria-hidden="true" size={14} />
                    {t("flow.connection.details.editConfiguration")}
                  </Button>
                  <Button
                    disabled={busy || !connection.enabled}
                    onClick={() => void store.test(connection.id)}
                    size="compact"
                    variant="secondary"
                  >
                    <RotateCw aria-hidden="true" size={14} />
                    {testing
                      ? t("flow.connection.details.testing")
                      : t("flow.connection.details.retest")}
                  </Button>
                </span>
              </header>
              <div className="connection-attention__issues">
                {problems.map((problem) => (
                  <div
                    className="connection-attention__issue"
                    key={problem.code}
                  >
                    <AlertTriangle aria-hidden="true" size={16} />
                    <span>
                      <small>
                        {connectionProblemAreaLabel(problem.area, language)}
                      </small>
                      <strong>{problem.title}</strong>
                      <span>{problem.detail}</span>
                    </span>
                  </div>
                ))}
              </div>
            </section>
          ) : null}
        </>
      ) : null}

      {variant !== "core" ? (
        <>
          {snapshot.error ? (
            <div
              className="connections-feedback connections-feedback--error"
              role="alert"
            >
              <AlertTriangle aria-hidden="true" size={16} />
              <span>{snapshot.error}</span>
              <Button
                onClick={() => store.clearFeedback()}
                size="compact"
                variant="quiet"
              >
                {t("flow.connection.close")}
              </Button>
            </div>
          ) : snapshot.notice ? (
            <div
              className="connections-feedback connections-feedback--success"
              role="status"
            >
              <CheckCircle2 aria-hidden="true" size={16} />
              <span>{connectionNoticeLabel(snapshot.notice, t)}</span>
              <Button
                onClick={() => store.clearFeedback()}
                size="compact"
                variant="quiet"
              >
                {t("flow.connection.close")}
              </Button>
            </div>
          ) : null}

          <dl className="connection-detail-list connection-detail-list--summary">
            <div>
              <dt>{t("flow.connection.details.account")}</dt>
              <dd>{connectionAccountLabel(connection, language)}</dd>
            </div>
            <div>
              <dt>{t("flow.connection.details.usageScope")}</dt>
              <dd>{usageSummary}</dd>
            </div>
            <div>
              <dt>{t("flow.connection.details.lastCheck")}</dt>
              <dd>{formatDate(connection.lastTestedAt, language)}</dd>
            </div>
            <div>
              <dt>{t("flow.connection.details.authorizationExpires")}</dt>
              <dd>{formatDate(connection.authContext.expiresAt, language)}</dd>
            </div>
          </dl>

          {health ? (
            <div className="connection-details__health">
              <ShieldCheck aria-hidden="true" size={16} />
              <span>
                <strong>{health.message}</strong>
                <small>{formatDate(health.checkedAt, language)}</small>
              </span>
            </div>
          ) : null}

          <details className="connection-technical-details">
            <summary>{t("flow.connection.details.technical")}</summary>
            <section>
              <h3>{t("flow.connection.details.identityAuthorization")}</h3>
              <DetailList
                items={[
                  [
                    t("flow.connection.details.provider"),
                    definition?.name ?? connection.integrationDefinitionId,
                  ],
                  [
                    t("flow.connection.details.type"),
                    definition
                      ? integrationKindLabel(definition.kind, language)
                      : "—",
                  ],
                  [
                    t("flow.connection.details.authMethod"),
                    definition
                      ? authSchemeLabel(definition.authScheme, language)
                      : "—",
                  ],
                  [
                    t("flow.connection.details.ownership"),
                    ownerTypeLabel(connection.ownerType, language),
                  ],
                  [
                    t("flow.connection.details.authStatus"),
                    authVerificationLabel(
                      connection.authContext.verification,
                      language,
                    ),
                  ],
                  [
                    t("flow.connection.details.tenant"),
                    accountValue(
                      connection,
                      "tenant",
                      t("flow.connection.details.undeclared"),
                    ),
                  ],
                  [
                    t("flow.connection.details.workspace"),
                    accountValue(
                      connection,
                      "workspace",
                      t("flow.connection.details.undeclared"),
                    ),
                  ],
                ]}
              />
              <ScopeList scopes={connection.authContext.grantedScopes} />
            </section>
            <section>
              <h3>{t("flow.connection.details.runtime")}</h3>
              <DetailList
                items={[
                  [
                    t("flow.connection.details.environment"),
                    connection.environment,
                  ],
                  [
                    t("flow.connection.details.runtime"),
                    server?.server.name ?? connection.runtimeBinding.serverId,
                  ],
                  [
                    t("flow.connection.details.runtimeStatus"),
                    server?.status.status ?? "unavailable",
                  ],
                  [
                    t("flow.connection.details.connectionRevision"),
                    String(connection.revision),
                  ],
                ]}
              />
              {connection.lastError ? (
                <div className="connection-details__runtime-error">
                  <AlertTriangle aria-hidden="true" size={14} />
                  <span>{connection.lastError}</span>
                </div>
              ) : null}
              {server?.server.envKeys.length ? (
                <div className="connection-details__runtime-note">
                  <Info aria-hidden="true" size={14} />
                  <span>{t("flow.connection.details.legacyRuntime")}</span>
                </div>
              ) : null}
            </section>
            <section>
              <header className="connection-capabilities__header">
                <h3>{t("flow.connection.details.capabilities")}</h3>
                {revisions && revisions.length > 0 ? (
                  <Select
                    label={t("flow.connection.details.capabilityRevision")}
                    onChange={setSelectedRevision}
                    options={revisions.map((item) => ({
                      value: String(item.revision),
                      label: `${t("flow.connection.details.revision")} ${item.revision}`,
                    }))}
                    value={
                      selectedRevision ||
                      String(defaultRevision?.revision ?? "")
                    }
                  />
                ) : null}
              </header>
              <CapabilityRevisionView
                revision={revision}
                revisions={revisions}
              />
            </section>
          </details>
        </>
      ) : null}
    </article>
  );
}

function DetailList({
  items,
}: {
  items: ReadonlyArray<readonly [string, string]>;
}) {
  return (
    <dl className="connection-detail-list">
      {items.map(([label, value]) => (
        <div key={label}>
          <dt>{label}</dt>
          <dd>{value}</dd>
        </div>
      ))}
    </dl>
  );
}

function ScopeList({ scopes }: { scopes: readonly string[] }) {
  const { t } = useApplicationLanguage();
  return (
    <div className="connection-scopes">
      <strong>{t("flow.connection.details.grantedScopes")}</strong>
      {scopes.length > 0 ? (
        <span>
          {scopes.map((scope) => (
            <Badge key={scope}>{scope}</Badge>
          ))}
        </span>
      ) : (
        <small>{t("flow.connection.details.noScopes")}</small>
      )}
    </div>
  );
}

function CapabilityRevisionView({
  revision,
  revisions,
}: {
  revision?: ConnectionCapabilityRevision;
  revisions?: readonly ConnectionCapabilityRevision[];
}) {
  const { language, t } = useApplicationLanguage();
  if (!revisions) {
    return (
      <div className="connections-inline-state">
        <Clock3 aria-hidden="true" size={16} />{" "}
        {t("flow.connection.details.loadingCapabilities")}
      </div>
    );
  }
  if (!revision) {
    return (
      <div className="connections-empty-state connections-empty-state--compact">
        <Wrench aria-hidden="true" size={18} />
        <strong>{t("flow.connection.details.noCapabilities")}</strong>
        <span>{t("flow.connection.details.noCapabilitiesDetail")}</span>
      </div>
    );
  }
  return (
    <div className="connection-capabilities__content">
      <div className="connection-capabilities__summary">
        <span>
          <Wrench aria-hidden="true" size={14} /> {revision.capabilities.length}{" "}
          {t("flow.connection.details.items")}
        </span>
        <span>
          <Server aria-hidden="true" size={14} /> {revision.source}
        </span>
        <span>
          <Clock3 aria-hidden="true" size={14} />{" "}
          {formatDate(revision.discoveredAt, language)}
        </span>
      </div>
      <div className="connection-capabilities__list">
        {revision.capabilities.map((capability) => (
          <article key={capability.capabilityId}>
            <span className="connections-icon">
              <Wrench aria-hidden="true" size={14} />
            </span>
            <span>
              <strong>{capability.displayName || capability.name}</strong>
              <code>{capability.name}</code>
              {capability.description ? (
                <small>{capability.description}</small>
              ) : null}
            </span>
            <span className="connection-capabilities__permissions">
              {capability.permissionLabels.map((label) => (
                <Badge key={label}>{label}</Badge>
              ))}
            </span>
          </article>
        ))}
      </div>
    </div>
  );
}

function accountValue(
  connection: Connection,
  kind: "tenant" | "workspace",
  undeclared: string,
) {
  const account = connection.authContext.account;
  if (kind === "tenant") {
    return account.tenantName || account.tenantId || undeclared;
  }
  return account.workspaceName || account.workspaceId || undeclared;
}

function describeUsage(
  usage: ConnectionDetailsProps["usage"],
  t: ReturnType<typeof useApplicationLanguage>["t"],
): string {
  if (!usage) return t("flow.connection.details.loadingUsage");
  const parts = [];
  if (usage.flowNames.length)
    parts.push(
      `${usage.flowNames.length} ${t("flow.connection.details.flowCount")}`,
    );
  if (usage.agentNames.length)
    parts.push(
      `${usage.agentNames.length} ${t("flow.connection.details.agentCount")}`,
    );
  return parts.join(" · ") || t("flow.connection.details.unused");
}

function formatDate(
  value: string | null | undefined,
  language: ApplicationLanguage,
): string {
  if (!value) return "—";
  const date = new Date(value);
  return Number.isNaN(date.valueOf()) ? value : date.toLocaleString(language);
}

function connectionNoticeLabel(
  notice: ConnectionNotice,
  t: ReturnType<typeof useApplicationLanguage>["t"],
): string {
  if (notice.kind === "created") return t("flow.connection.notice.created");
  if (notice.kind === "updated") return t("flow.connection.notice.updated");
  if (notice.kind === "test_passed") {
    return t("flow.connection.notice.testPassed");
  }
  if (notice.kind === "capabilities_unchanged") {
    return `${t("flow.connection.notice.unchanged")} ${notice.count} ${t("flow.connection.details.items")}`;
  }
  return `${t("flow.connection.notice.snapshotUpdated")} ${notice.added}, ${t("flow.connection.notice.removed")} ${notice.removed}, ${t("flow.connection.notice.changed")} ${notice.changed}`;
}
