import type { BadgeVariant } from "../ui";
import {
  defaultApplicationLanguage,
  interfaceMessage,
  type ApplicationLanguage,
  type InterfaceMessageKey,
} from "../../applicationLanguage.ts";
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

export function connectionStatusLabel(
  status: ConnectionStatus,
  language: ApplicationLanguage = defaultApplicationLanguage,
): string {
  if (status === "ready")
    return message(language, "flow.connection.status.ready");
  if (status === "configured")
    return message(language, "flow.connection.status.configured");
  if (status === "degraded")
    return message(language, "flow.connection.status.degraded");
  if (status === "reauth_required")
    return message(language, "flow.connection.status.reauthRequired");
  return message(language, "flow.connection.status.disabled");
}

export function connectionStatusVariant(
  status: ConnectionStatus,
): BadgeVariant {
  if (status === "ready") return "success";
  if (status === "configured") return "info";
  if (status === "degraded" || status === "reauth_required") return "warning";
  return "neutral";
}

export function integrationKindLabel(
  kind: IntegrationKind,
  language: ApplicationLanguage = defaultApplicationLanguage,
): string {
  if (kind === "mcp") return "MCP";
  if (kind === "oauth_api") return "OAuth API";
  if (kind === "database")
    return message(language, "flow.connection.kind.database");
  return message(language, "flow.connection.kind.localApp");
}

export function authSchemeLabel(
  scheme: IntegrationAuthScheme,
  language: ApplicationLanguage = defaultApplicationLanguage,
): string {
  if (scheme === "api_key") return "API Key";
  if (scheme === "oauth2") return "OAuth 2.0";
  if (scheme === "external")
    return message(language, "flow.connection.auth.external");
  return message(language, "flow.connection.auth.none");
}

export function authVerificationLabel(
  verification: ConnectionAuthVerification,
  language: ApplicationLanguage = defaultApplicationLanguage,
): string {
  if (verification === "verified")
    return message(language, "flow.connection.verification.verified");
  if (verification === "not_required")
    return message(language, "flow.connection.verification.notRequired");
  if (verification === "legacy_unverified")
    return message(language, "flow.connection.verification.legacyUnverified");
  return message(language, "flow.connection.verification.unverified");
}

export function ownerTypeLabel(
  owner: ConnectionOwnerType,
  language: ApplicationLanguage = defaultApplicationLanguage,
): string {
  if (owner === "org_shared")
    return message(language, "flow.connection.owner.orgShared");
  if (owner === "service_account")
    return message(language, "flow.connection.owner.serviceAccount");
  return message(language, "flow.connection.owner.personal");
}

export function connectionAccountLabel(
  connection: Connection,
  language: ApplicationLanguage = defaultApplicationLanguage,
): string {
  const account = connection.authContext.account;
  return (
    account.displayName ||
    account.externalAccountId ||
    account.workspaceName ||
    account.tenantName ||
    message(language, "flow.connection.accountUndeclared")
  );
}

export function connectionProblems(
  connection: Connection,
  language: ApplicationLanguage = defaultApplicationLanguage,
): ConnectionProblem[] {
  const problems: ConnectionProblem[] = [];

  if (!connection.enabled || connection.status === "disabled") {
    problems.push({
      code: "disabled",
      area: "configuration",
      title: message(language, "flow.connection.problem.disabled.title"),
      detail: message(language, "flow.connection.problem.disabled.detail"),
    });
  } else if (connection.status === "configured") {
    problems.push({
      code: "configured",
      area: "runtime",
      title: message(language, "flow.connection.problem.configured.title"),
      detail: message(language, "flow.connection.problem.configured.detail"),
    });
  } else if (connection.status === "degraded") {
    problems.push({
      code: "degraded",
      area: "runtime",
      title: message(language, "flow.connection.problem.degraded.title"),
      detail: connection.lastError
        ? `${message(language, "flow.connection.problem.degraded.errorPrefix")}${connection.lastError}`
        : message(language, "flow.connection.problem.degraded.detail"),
    });
  } else if (connection.status === "reauth_required") {
    problems.push({
      code: "reauth_required",
      area: "authentication",
      title: message(language, "flow.connection.problem.reauth.title"),
      detail: message(language, "flow.connection.problem.reauth.detail"),
    });
  }

  if (connection.authContext.verification === "unverified") {
    problems.push({
      code: "unverified",
      area: "authentication",
      title: message(language, "flow.connection.problem.unverified.title"),
      detail: message(language, "flow.connection.problem.unverified.detail"),
    });
  } else if (connection.authContext.verification === "legacy_unverified") {
    problems.push({
      code: "legacy_unverified",
      area: "authentication",
      title: message(language, "flow.connection.problem.legacy.title"),
      detail: message(language, "flow.connection.problem.legacy.detail"),
    });
  }

  return problems;
}

export function connectionProblemAreaLabel(
  area: ConnectionProblem["area"],
  language: ApplicationLanguage = defaultApplicationLanguage,
): string {
  if (area === "configuration")
    return message(language, "flow.connection.problem.area.configuration");
  if (area === "runtime")
    return message(language, "flow.connection.problem.area.runtime");
  return message(language, "flow.connection.problem.area.authentication");
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
  language: ApplicationLanguage = defaultApplicationLanguage,
): ConnectionFormErrors {
  const errors: ConnectionFormErrors = {};
  if (!values.integrationDefinitionId)
    errors.definition = message(
      language,
      "flow.connection.validation.definition",
    );
  if (!values.name.trim())
    errors.name = message(language, "flow.connection.validation.name");
  if (!values.environment.trim())
    errors.environment = message(
      language,
      "flow.connection.validation.environment",
    );
  if (!values.serverId)
    errors.server = message(language, "flow.connection.validation.server");
  else if (reservedServerIds.has(values.serverId)) {
    errors.server = message(
      language,
      "flow.connection.validation.serverReserved",
    );
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

function message(
  language: ApplicationLanguage,
  key: InterfaceMessageKey,
): string {
  return interfaceMessage(language, key);
}
