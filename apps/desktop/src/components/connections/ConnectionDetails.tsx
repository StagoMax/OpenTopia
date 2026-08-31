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
import type { ConnectionsSnapshot, ConnectionsStore } from "./store";

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
  const problems = connectionProblems(connection);
  const usageSummary = describeUsage(usage);

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
                {connectionStatusLabel(connection.status)}
              </Badge>
            </span>
            <p>
              {definition?.name ?? "未知服务"} ·{" "}
              {connectionAccountLabel(connection)}
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
                {definition?.name ?? "未知服务"} ·{" "}
                {connectionAccountLabel(connection)}
              </small>
              <span>
                <h2>{connection.name}</h2>
                <Badge variant={connectionStatusVariant(connection.status)}>
                  {connectionStatusLabel(connection.status)}
                </Badge>
              </span>
              <p>
                {problems[0]?.detail ??
                  `${usageSummary}，当前可以供已授权的流程使用。`}
              </p>
            </div>
          </section>

          <dl className="connection-operation-facts" aria-label="连接摘要">
            <div>
              <dt>使用范围</dt>
              <dd>{usageSummary}</dd>
            </div>
            <div>
              <dt>最近检查</dt>
              <dd>{formatDate(connection.lastTestedAt)}</dd>
            </div>
            <div>
              <dt>可用能力</dt>
              <dd>
                {revision ? `${revision.capabilities.length} 项` : "尚未发现"}
              </dd>
            </div>
          </dl>

          {problems.length > 0 ? (
            <section
              className="connection-attention"
              aria-label="需要处理的问题"
            >
              <header>
                <strong>需要处理</strong>
                <span className="connection-attention__actions">
                  <Button
                    disabled={busy}
                    onClick={() => store.beginEdit()}
                    size="compact"
                    variant="quiet"
                  >
                    <Pencil aria-hidden="true" size={14} />
                    编辑配置
                  </Button>
                  <Button
                    disabled={busy || !connection.enabled}
                    onClick={() => void store.test(connection.id)}
                    size="compact"
                    variant="secondary"
                  >
                    <RotateCw aria-hidden="true" size={14} />
                    {testing ? "测试中…" : "重新测试"}
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
                      <small>{connectionProblemAreaLabel(problem.area)}</small>
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
                关闭
              </Button>
            </div>
          ) : snapshot.notice ? (
            <div
              className="connections-feedback connections-feedback--success"
              role="status"
            >
              <CheckCircle2 aria-hidden="true" size={16} />
              <span>{snapshot.notice}</span>
              <Button
                onClick={() => store.clearFeedback()}
                size="compact"
                variant="quiet"
              >
                关闭
              </Button>
            </div>
          ) : null}

          <dl className="connection-detail-list connection-detail-list--summary">
            <div>
              <dt>账号</dt>
              <dd>{connectionAccountLabel(connection)}</dd>
            </div>
            <div>
              <dt>使用范围</dt>
              <dd>{usageSummary}</dd>
            </div>
            <div>
              <dt>最近检查</dt>
              <dd>{formatDate(connection.lastTestedAt)}</dd>
            </div>
            <div>
              <dt>授权到期</dt>
              <dd>{formatDate(connection.authContext.expiresAt)}</dd>
            </div>
          </dl>

          {health ? (
            <div className="connection-details__health">
              <ShieldCheck aria-hidden="true" size={16} />
              <span>
                <strong>{health.message}</strong>
                <small>{formatDate(health.checkedAt)}</small>
              </span>
            </div>
          ) : null}

          <details className="connection-technical-details">
            <summary>技术配置与能力</summary>
            <section>
              <h3>身份与授权</h3>
              <DetailList
                items={[
                  [
                    "Provider",
                    definition?.name ?? connection.integrationDefinitionId,
                  ],
                  [
                    "类型",
                    definition ? integrationKindLabel(definition.kind) : "—",
                  ],
                  [
                    "认证方式",
                    definition ? authSchemeLabel(definition.authScheme) : "—",
                  ],
                  ["所有权", ownerTypeLabel(connection.ownerType)],
                  [
                    "认证状态",
                    authVerificationLabel(connection.authContext.verification),
                  ],
                  ["Tenant", accountValue(connection, "tenant")],
                  ["Workspace", accountValue(connection, "workspace")],
                ]}
              />
              <ScopeList scopes={connection.authContext.grantedScopes} />
            </section>
            <section>
              <h3>Runtime</h3>
              <DetailList
                items={[
                  ["环境", connection.environment],
                  [
                    "Runtime",
                    server?.server.name ?? connection.runtimeBinding.serverId,
                  ],
                  ["Runtime 状态", server?.status.status ?? "unavailable"],
                  ["Connection revision", String(connection.revision)],
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
                  <span>该 runtime 仍使用旧版环境变量凭据，来源尚未验证。</span>
                </div>
              ) : null}
            </section>
            <section>
              <header className="connection-capabilities__header">
                <h3>能力</h3>
                {revisions && revisions.length > 0 ? (
                  <Select
                    label="Capability revision"
                    onChange={setSelectedRevision}
                    options={revisions.map((item) => ({
                      value: String(item.revision),
                      label: `Revision ${item.revision}`,
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
  return (
    <div className="connection-scopes">
      <strong>已授予范围</strong>
      {scopes.length > 0 ? (
        <span>
          {scopes.map((scope) => (
            <Badge key={scope}>{scope}</Badge>
          ))}
        </span>
      ) : (
        <small>Provider 未报告账号级 scope。</small>
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
  if (!revisions) {
    return (
      <div className="connections-inline-state">
        <Clock3 aria-hidden="true" size={16} /> 正在读取能力…
      </div>
    );
  }
  if (!revision) {
    return (
      <div className="connections-empty-state connections-empty-state--compact">
        <Wrench aria-hidden="true" size={18} />
        <strong>尚未发现能力</strong>
        <span>测试连接后刷新能力，即可读取可用操作。</span>
      </div>
    );
  }
  return (
    <div className="connection-capabilities__content">
      <div className="connection-capabilities__summary">
        <span>
          <Wrench aria-hidden="true" size={14} /> {revision.capabilities.length}{" "}
          项
        </span>
        <span>
          <Server aria-hidden="true" size={14} /> {revision.source}
        </span>
        <span>
          <Clock3 aria-hidden="true" size={14} />{" "}
          {formatDate(revision.discoveredAt)}
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

function accountValue(connection: Connection, kind: "tenant" | "workspace") {
  const account = connection.authContext.account;
  if (kind === "tenant") {
    return account.tenantName || account.tenantId || "未声明";
  }
  return account.workspaceName || account.workspaceId || "未声明";
}

function describeUsage(usage: ConnectionDetailsProps["usage"]): string {
  if (!usage) return "正在读取使用范围";
  const parts = [];
  if (usage.flowNames.length) parts.push(`${usage.flowNames.length} 个 Flow`);
  if (usage.agentNames.length)
    parts.push(`${usage.agentNames.length} 个 Agent`);
  return parts.join(" · ") || "尚未使用";
}

function formatDate(value?: string | null): string {
  if (!value) return "—";
  const date = new Date(value);
  return Number.isNaN(date.valueOf()) ? value : date.toLocaleString();
}
