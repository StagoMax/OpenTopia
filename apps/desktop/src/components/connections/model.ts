import type { BadgeVariant } from "../ui";
import type {
  Connection,
  ConnectionInput,
  ConnectionAuthVerification,
  ConnectionOwnerType,
  ConnectionStatus,
  ConnectionUpdate,
  IntegrationAuthScheme,
  IntegrationDefinition,
  IntegrationKind,
} from "../../types";

export type ConnectionFormValues = {
  integrationDefinitionId: string;
  name: string;
  ownerType: ConnectionOwnerType;
  environment: string;
  serverId: string;
  enabled: boolean;
  credentialRef: string;
  accountDisplayName: string;
  externalAccountId: string;
  tenantId: string;
  tenantName: string;
  workspaceId: string;
  workspaceName: string;
  grantedScopes: string;
  expiresAt: string;
};

export type ConnectionFormErrors = Partial<
  Record<"definition" | "name" | "environment" | "server", string>
>;

export type ConnectionProblem = {
  code:
    | "disabled"
    | "configured"
    | "degraded"
    | "reauth_required"
    | "unverified"
    | "legacy_unverified";
  area: "configuration" | "runtime" | "authentication";
  title: string;
  detail: string;
};

const STATUS_PRIORITY: Record<ConnectionStatus, number> = {
  reauth_required: 0,
  degraded: 1,
  configured: 2,
  ready: 3,
  disabled: 4,
};

export function sortConnections(
  connections: readonly Connection[],
): Connection[] {
  return [...connections].sort(
    (left, right) =>
      STATUS_PRIORITY[left.status] - STATUS_PRIORITY[right.status] ||
      left.name.localeCompare(right.name),
  );
}

export function connectionStatusLabel(status: ConnectionStatus): string {
  if (status === "ready") return "可用";
  if (status === "configured") return "待测试";
  if (status === "degraded") return "异常";
  if (status === "reauth_required") return "需重新授权";
  return "已停用";
}

export function connectionStatusVariant(
  status: ConnectionStatus,
): BadgeVariant {
  if (status === "ready") return "success";
  if (status === "configured") return "info";
  if (status === "degraded" || status === "reauth_required") return "warning";
  return "neutral";
}

export function integrationKindLabel(kind: IntegrationKind): string {
  if (kind === "mcp") return "MCP";
  if (kind === "oauth_api") return "OAuth API";
  if (kind === "database") return "Database";
  return "Local App";
}

export function authSchemeLabel(scheme: IntegrationAuthScheme): string {
  if (scheme === "api_key") return "API Key";
  if (scheme === "oauth2") return "OAuth 2.0";
  if (scheme === "external") return "外部授权";
  return "无需登录";
}

export function authVerificationLabel(
  verification: ConnectionAuthVerification,
): string {
  if (verification === "verified") return "已验证";
  if (verification === "not_required") return "无需认证";
  if (verification === "legacy_unverified") return "Legacy · 未验证";
  return "未验证";
}

export function ownerTypeLabel(owner: ConnectionOwnerType): string {
  if (owner === "org_shared") return "组织共享";
  if (owner === "service_account") return "服务账号";
  return "个人账号";
}

export function connectionAccountLabel(connection: Connection): string {
  const account = connection.authContext.account;
  return (
    account.displayName ||
    account.externalAccountId ||
    account.workspaceName ||
    account.tenantName ||
    "未声明账号"
  );
}

export function connectionProblems(
  connection: Connection,
): ConnectionProblem[] {
  const problems: ConnectionProblem[] = [];

  if (!connection.enabled || connection.status === "disabled") {
    problems.push({
      code: "disabled",
      area: "configuration",
      title: "Connection 已停用",
      detail: "启用此 Connection 后，外部调用才能通过执行边界。",
    });
  } else if (connection.status === "configured") {
    problems.push({
      code: "configured",
      area: "runtime",
      title: "尚未完成连接测试",
      detail: "当前仅完成配置；请测试 runtime 与账号后再刷新能力。",
    });
  } else if (connection.status === "degraded") {
    problems.push({
      code: "degraded",
      area: "runtime",
      title: "运行状态异常",
      detail: connection.lastError
        ? `最近错误：${connection.lastError}`
        : "检查 runtime 与凭据配置，然后重新测试此 Connection。",
    });
  } else if (connection.status === "reauth_required") {
    problems.push({
      code: "reauth_required",
      area: "authentication",
      title: "账号授权已失效",
      detail: "更新凭据或重新授权后，再测试此 Connection。",
    });
  }

  if (connection.authContext.verification === "unverified") {
    problems.push({
      code: "unverified",
      area: "authentication",
      title: "认证尚未验证",
      detail: "当前凭据未通过账号验证，外部调用会被拒绝。",
    });
  } else if (connection.authContext.verification === "legacy_unverified") {
    problems.push({
      code: "legacy_unverified",
      area: "authentication",
      title: "Legacy 认证来源未验证",
      detail: "旧版环境变量凭据未验证来源，需要迁移或重新验证。",
    });
  }

  return problems;
}

export function connectionProblemAreaLabel(
  area: ConnectionProblem["area"],
): string {
  if (area === "configuration") return "配置";
  if (area === "runtime") return "运行";
  return "认证";
}

export function definitionForConnection(
  definitions: readonly IntegrationDefinition[],
  connection: Connection,
): IntegrationDefinition | undefined {
  return definitions.find(
    (definition) => definition.id === connection.integrationDefinitionId,
  );
}

export function emptyConnectionForm(
  definitions: readonly IntegrationDefinition[],
): ConnectionFormValues {
  const firstMcp = definitions.find(
    (definition) => definition.kind === "mcp" && definition.enabled,
  );
  return {
    integrationDefinitionId: firstMcp?.id ?? "",
    name: "",
    ownerType: "personal",
    environment: "production",
    serverId: "",
    enabled: true,
    credentialRef: "",
    accountDisplayName: "",
    externalAccountId: "",
    tenantId: "",
    tenantName: "",
    workspaceId: "",
    workspaceName: "",
    grantedScopes: "",
    expiresAt: "",
  };
}

export function connectionFormFromConnection(
  connection: Connection,
): ConnectionFormValues {
  const account = connection.authContext.account;
  return {
    integrationDefinitionId: connection.integrationDefinitionId,
    name: connection.name,
    ownerType: connection.ownerType,
    environment: connection.environment,
    serverId: connection.runtimeBinding.serverId,
    enabled: connection.enabled,
    credentialRef: connection.authContext.credentialRef ?? "",
    accountDisplayName: account.displayName ?? "",
    externalAccountId: account.externalAccountId ?? "",
    tenantId: account.tenantId ?? "",
    tenantName: account.tenantName ?? "",
    workspaceId: account.workspaceId ?? "",
    workspaceName: account.workspaceName ?? "",
    grantedScopes: connection.authContext.grantedScopes.join(", "),
    expiresAt: connection.authContext.expiresAt ?? "",
  };
}

export function validateConnectionForm(
  values: ConnectionFormValues,
  reservedServerIds: ReadonlySet<string>,
): ConnectionFormErrors {
  const errors: ConnectionFormErrors = {};
  if (!values.integrationDefinitionId)
    errors.definition = "请选择 Provider 定义";
  if (!values.name.trim()) errors.name = "请输入 Connection 名称";
  if (!values.environment.trim()) errors.environment = "请输入运行环境";
  if (!values.serverId) errors.server = "请选择独立的 MCP runtime";
  else if (reservedServerIds.has(values.serverId)) {
    errors.server = "该 MCP runtime 已绑定其他 Connection";
  }
  return errors;
}

export function connectionInputFromForm(
  values: ConnectionFormValues,
): ConnectionInput {
  return {
    integrationDefinitionId: values.integrationDefinitionId,
    name: values.name.trim(),
    ownerType: values.ownerType,
    environment: values.environment.trim(),
    enabled: values.enabled,
    runtimeBinding: { kind: "mcp_server", serverId: values.serverId },
    authContext: {
      credentialRef: nullable(values.credentialRef),
      account: {
        displayName: nullable(values.accountDisplayName),
        externalAccountId: nullable(values.externalAccountId),
        tenantId: nullable(values.tenantId),
        tenantName: nullable(values.tenantName),
        workspaceId: nullable(values.workspaceId),
        workspaceName: nullable(values.workspaceName),
      },
      grantedScopes: Array.from(
        new Set(
          values.grantedScopes
            .split(",")
            .map((scope) => scope.trim())
            .filter(Boolean),
        ),
      ),
      expiresAt: nullable(values.expiresAt),
    },
  };
}

export function connectionUpdateFromInput(
  current: Connection,
  input: ConnectionInput,
): ConnectionUpdate {
  const update: ConnectionUpdate = { expectedRevision: current.revision };
  if (current.name !== input.name) update.name = input.name;
  if (current.ownerType !== input.ownerType) update.ownerType = input.ownerType;
  if (current.environment !== input.environment) {
    update.environment = input.environment;
  }
  if (current.enabled !== input.enabled) update.enabled = input.enabled;
  if (current.runtimeBinding.serverId !== input.runtimeBinding.serverId) {
    update.runtimeBinding = input.runtimeBinding;
  }
  const currentAuth = {
    credentialRef: current.authContext.credentialRef ?? null,
    account: normalizedAccount(current.authContext.account),
    grantedScopes: current.authContext.grantedScopes,
    expiresAt: current.authContext.expiresAt ?? null,
  };
  const inputAuth = {
    credentialRef: input.authContext.credentialRef ?? null,
    account: normalizedAccount(input.authContext.account),
    grantedScopes: input.authContext.grantedScopes,
    expiresAt: input.authContext.expiresAt ?? null,
  };
  if (JSON.stringify(currentAuth) !== JSON.stringify(inputAuth)) {
    update.authContext = input.authContext;
  }
  return update;
}

function nullable(value: string): string | null {
  const normalized = value.trim();
  return normalized || null;
}

function normalizedAccount(account: Connection["authContext"]["account"]) {
  return {
    displayName: account.displayName ?? null,
    externalAccountId: account.externalAccountId ?? null,
    tenantId: account.tenantId ?? null,
    tenantName: account.tenantName ?? null,
    workspaceId: account.workspaceId ?? null,
    workspaceName: account.workspaceName ?? null,
  };
}
