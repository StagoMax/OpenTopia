import {
  AlertTriangle,
  Cable,
  CheckCircle2,
  Clock3,
  Info,
  KeyRound,
  Pencil,
  RefreshCw,
  RotateCw,
  Server,
  ShieldCheck,
  UserRound,
  Wrench,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type {
  Connection,
  ConnectionCapabilityRevision,
  IntegrationDefinition,
  McpServerView,
} from "../../types";
import { Badge, Button, Panel, Select } from "../ui";
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
  variant?: "full" | "core" | "inspector";
};

export function ConnectionDetails({
  connection,
  definition,
  snapshot,
  store,
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
  const refreshing = snapshot.busyAction === `refresh:${connection.id}`;
  const busy = Boolean(snapshot.busyAction);
  const health =
    snapshot.lastHealth?.connectionId === connection.id
      ? snapshot.lastHealth.health
      : null;
  const problems = connectionProblems(connection);

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
              {definition?.name ?? "Unknown provider"} ·{" "}
              {connection.environment} · {connectionAccountLabel(connection)}
            </p>
          </span>
          <div className="connection-details__actions">
            <Button
              disabled={busy}
              onClick={() => store.beginEdit()}
              size="compact"
              variant="quiet"
            >
              <Pencil aria-hidden="true" size={14} /> 编辑
            </Button>
            <Button
              disabled={busy || !connection.enabled}
              onClick={() => void store.test(connection.id)}
              size="compact"
              variant="secondary"
            >
              <RotateCw aria-hidden="true" size={14} />
              {testing ? "测试中…" : "测试连接"}
            </Button>
            <Button
              disabled={busy || connection.status !== "ready"}
              onClick={() => void store.refreshCapabilities(connection.id)}
              size="compact"
              variant="primary"
            >
              <RefreshCw aria-hidden="true" size={14} />
              {refreshing ? "发现中…" : "刷新能力"}
            </Button>
          </div>
        </header>
      ) : null}

      {variant === "core" && problems.length > 0 ? (
        <Panel
          actions={
            <div className="connection-attention__actions">
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
                {testing ? "测试中…" : "测试连接"}
              </Button>
            </div>
          }
          className="connection-attention"
          title={`${connection.name} · 需要处理`}
        >
          <div className="connection-attention__issues">
            {problems.map((problem) => (
              <div className="connection-attention__issue" key={problem.code}>
                <AlertTriangle aria-hidden="true" size={16} />
                <span>
                  <small>{connectionProblemAreaLabel(problem.area)}</small>
                  <strong>{problem.title}</strong>
                  <span>{problem.detail}</span>
                </span>
              </div>
            ))}
          </div>
        </Panel>
      ) : null}

      {variant !== "core" && connection.status === "reauth_required" ? (
        <div className="connections-feedback connections-feedback--warning">
          <KeyRound aria-hidden="true" size={16} />
          <span>
            <strong>账号授权已失效</strong>
            <small>
              当前切片仅标记重新授权状态；OAuth/API Key 登录流程将在后续
              Provider 接入阶段开放。
            </small>
          </span>
          <Button disabled size="compact" variant="secondary">
            重新授权
          </Button>
        </div>
      ) : null}

      {variant !== "core" && snapshot.error ? (
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
      ) : variant !== "core" && snapshot.notice ? (
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

      {variant !== "core" ? (
        <div className="connection-details__grid">
          <Panel title="Identity & access / 身份与授权">
            <DetailList
              items={[
                [
                  "Provider",
                  definition?.name ?? connection.integrationDefinitionId,
                ],
                [
                  "Provider kind",
                  definition ? integrationKindLabel(definition.kind) : "—",
                ],
                [
                  "Auth scheme",
                  definition ? authSchemeLabel(definition.authScheme) : "—",
                ],
                ["Owner", ownerTypeLabel(connection.ownerType)],
                ["Account", connectionAccountLabel(connection)],
                [
                  "Auth verification",
                  authVerificationLabel(connection.authContext.verification),
                ],
                ["Tenant", accountValue(connection, "tenant")],
                ["Workspace", accountValue(connection, "workspace")],
                [
                  "Credential",
                  connection.authContext.credentialRef
                    ? "Secret reference 已绑定"
                    : "无",
                ],
              ]}
            />
            <ScopeList scopes={connection.authContext.grantedScopes} />
          </Panel>

          <Panel title="Runtime & health / 运行与健康">
            <DetailList
              items={[
                ["Environment", connection.environment],
                [
                  "Runtime binding",
                  server?.server.name ?? connection.runtimeBinding.serverId,
                ],
                ["Runtime status", server?.status.status ?? "unavailable"],
                ["Connection revision", String(connection.revision)],
                ["Last tested", formatDate(connection.lastTestedAt)],
                ["Token expires", formatDate(connection.authContext.expiresAt)],
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
                <span>
                  该 runtime 仍从 legacy envKeys 读取凭据，来源尚未验证；runtime
                  ready 不代表账号认证健康。
                </span>
              </div>
            ) : null}
            {health ? (
              <div className="connection-details__health">
                <ShieldCheck aria-hidden="true" size={16} />
                <span>
                  <strong>{health.message}</strong>
                  <small>
                    runtime {health.runtimeStatus} · auth{" "}
                    {authVerificationLabel(health.authStatus)} ·{" "}
                    {health.toolsCount} tools · {formatDate(health.checkedAt)}
                  </small>
                </span>
              </div>
            ) : null}
          </Panel>
        </div>
      ) : null}

      {variant !== "inspector" ? (
        <Panel
          actions={
            revisions && revisions.length > 0 ? (
              <Select
                label="Capability revision"
                onChange={setSelectedRevision}
                options={revisions.map((item) => ({
                  value: String(item.revision),
                  label: `Revision ${item.revision}`,
                }))}
                value={
                  selectedRevision || String(defaultRevision?.revision ?? "")
                }
              />
            ) : undefined
          }
          className="connection-capabilities"
          title="Capability revision / 能力快照"
        >
          <CapabilityRevisionView revision={revision} revisions={revisions} />
        </Panel>
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
      <strong>Granted scopes / 已授予范围</strong>
      {scopes.length > 0 ? (
        <span>
          {scopes.map((scope) => (
            <Badge key={scope}>{scope}</Badge>
          ))}
        </span>
      ) : (
        <small>Provider 未报告账号级 scope。模板发布时仍需配置操作授权。</small>
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
        <Clock3 aria-hidden="true" size={16} /> 正在读取能力历史…
      </div>
    );
  }
  if (!revision) {
    return (
      <div className="connections-empty-state connections-empty-state--compact">
        <Wrench aria-hidden="true" size={18} />
        <strong>尚未发现能力</strong>
        <span>
          连接测试通过后执行“刷新能力”，系统将固化 MCP tools/list
          的不可变快照；resources 与 prompts 尚不支持。
        </span>
      </div>
    );
  }
  return (
    <div className="connection-capabilities__content">
      <div className="connection-capabilities__notice">
        <Info aria-hidden="true" size={14} />
        <span>
          {`本次快照范围：Tools ${discoveryCoverageLabel(revision.discoveryCoverage.tools)}；Resources ${discoveryCoverageLabel(revision.discoveryCoverage.resources)}；Prompts ${discoveryCoverageLabel(revision.discoveryCoverage.prompts)}。`}
        </span>
      </div>
      <div className="connection-capabilities__summary">
        <span>
          <Wrench aria-hidden="true" size={14} /> {revision.capabilities.length}{" "}
          tools
        </span>
        <span>
          <Server aria-hidden="true" size={14} /> {revision.source}
        </span>
        <span>
          <Clock3 aria-hidden="true" size={14} />{" "}
          {formatDate(revision.discoveredAt)}
        </span>
        <code>{revision.contentHash.slice(0, 12)}</code>
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

function discoveryCoverageLabel(support: "supported" | "unsupported"): string {
  return support === "supported" ? "已读取" : "暂未读取";
}

function accountValue(connection: Connection, kind: "tenant" | "workspace") {
  const account = connection.authContext.account;
  if (kind === "tenant")
    return account.tenantName || account.tenantId || "未声明";
  return account.workspaceName || account.workspaceId || "未声明";
}

function formatDate(value?: string | null): string {
  if (!value) return "—";
  const date = new Date(value);
  return Number.isNaN(date.valueOf()) ? value : date.toLocaleString();
}
