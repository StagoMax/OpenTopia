import type {
  AgentConnectionBinding,
  Connection,
  ConnectionCapability,
  ConnectionCapabilityRevision,
  IntegrationDefinition,
} from "../../types";

export type ConnectionGrantEligibility = {
  selectable: boolean;
  warning: boolean;
  reason: string | null;
};

export type ConnectionBindingFreshness = {
  state: "current" | "stale" | "unavailable";
  changedOperationIds: string[];
  removedOperationIds: string[];
};

export function normalizeConnectionBindings(
  bindings: readonly AgentConnectionBinding[] | undefined,
): AgentConnectionBinding[] {
  return bindings ? [...bindings] : [];
}

export function hasLegacyMcpProjection(
  allowAllMcpServers: boolean,
  mcpServerIds: readonly string[],
): boolean {
  return allowAllMcpServers || mcpServerIds.length > 0;
}

export function connectionGrantEligibility(
  connection: Connection,
  definition?: IntegrationDefinition | null,
  nowMs = Date.now(),
): ConnectionGrantEligibility {
  if (definition === null) {
    return {
      selectable: false,
      warning: false,
      reason: "Connection 的 Provider 定义不存在",
    };
  }
  if (definition && !definition.enabled) {
    return {
      selectable: false,
      warning: false,
      reason: "Connection 的 Provider 定义已停用",
    };
  }
  if (!connection.enabled || connection.status === "disabled") {
    return {
      selectable: false,
      warning: false,
      reason: "Connection 已停用，请先在 Connections 中启用并测试",
    };
  }
  if (connection.status !== "ready") {
    const reason = {
      configured: "Connection 尚未通过测试",
      degraded: "Connection 运行异常，请先修复健康检查",
      reauth_required: "Connection 需要重新授权",
      disabled: "Connection 已停用",
      ready: null,
    }[connection.status];
    return { selectable: false, warning: false, reason };
  }
  if (connection.activeCapabilityRevision == null) {
    return {
      selectable: false,
      warning: false,
      reason: "尚无能力快照，请先在 Connections 中刷新能力",
    };
  }
  if (
    connection.authContext.expiresAt != null &&
    Date.parse(connection.authContext.expiresAt) <= nowMs
  ) {
    return {
      selectable: false,
      warning: false,
      reason: "Connection 登录已过期，请先重新授权",
    };
  }
  if (connection.authContext.verification === "legacy_unverified") {
    const migratedLegacy =
      connection.authContext.credentialRef == null &&
      definition?.key.startsWith("legacy-mcp-");
    if (!migratedLegacy) {
      return {
        selectable: false,
        warning: false,
        reason: "Connection 使用未验证凭据，必须先重新授权",
      };
    }
    return {
      selectable: true,
      warning: true,
      reason: "Legacy 凭据未验证；发布前请确认账号与权限范围",
    };
  }
  if (
    connection.authContext.verification !== "verified" &&
    connection.authContext.verification !== "not_required"
  ) {
    return {
      selectable: false,
      warning: false,
      reason: "Connection 账号尚未验证",
    };
  }
  return { selectable: true, warning: false, reason: null };
}

export function replaceConnectionBinding(
  bindings: readonly AgentConnectionBinding[],
  next: AgentConnectionBinding,
): AgentConnectionBinding[] {
  const normalized = normalizeBinding(next);
  return [
    ...bindings.filter((binding) => binding.connectionId !== next.connectionId),
    normalized,
  ];
}

export function removeConnectionBinding(
  bindings: readonly AgentConnectionBinding[],
  connectionId: string,
): AgentConnectionBinding[] {
  return bindings.filter((binding) => binding.connectionId !== connectionId);
}

export function toggleOperationGrant(
  binding: AgentConnectionBinding,
  operationId: string,
): AgentConnectionBinding {
  const current = new Set(
    binding.operationGrants.map((grant) => grant.operationId),
  );
  if (current.has(operationId)) current.delete(operationId);
  else current.add(operationId);
  return normalizeBinding({
    ...binding,
    operationGrants: [...current].map((id) => ({ operationId: id })),
  });
}

export function bindingFreshness(
  binding: AgentConnectionBinding,
  pinnedRevision: ConnectionCapabilityRevision | undefined,
  activeRevision: ConnectionCapabilityRevision | undefined,
): ConnectionBindingFreshness {
  if (!pinnedRevision || !activeRevision) {
    return {
      state: "unavailable",
      changedOperationIds: [],
      removedOperationIds: binding.operationGrants.map(
        (grant) => grant.operationId,
      ),
    };
  }

  const pinnedById = capabilityMap(pinnedRevision.capabilities);
  const activeById = capabilityMap(activeRevision.capabilities);
  const changedOperationIds: string[] = [];
  const removedOperationIds: string[] = [];
  for (const grant of binding.operationGrants) {
    const pinned = pinnedById.get(grant.operationId);
    const active = activeById.get(grant.operationId);
    if (!pinned || !active) {
      removedOperationIds.push(grant.operationId);
    } else if (operationFingerprint(pinned) !== operationFingerprint(active)) {
      changedOperationIds.push(grant.operationId);
    }
  }
  return {
    state:
      changedOperationIds.length > 0 || removedOperationIds.length > 0
        ? "stale"
        : "current",
    changedOperationIds,
    removedOperationIds,
  };
}

export function rebaseBinding(
  binding: AgentConnectionBinding,
  activeRevision: ConnectionCapabilityRevision,
): AgentConnectionBinding {
  const available = new Set(
    activeRevision.capabilities.map((capability) => capability.capabilityId),
  );
  return normalizeBinding({
    ...binding,
    capabilityRevision: activeRevision.revision,
    operationGrants: binding.operationGrants.filter((grant) =>
      available.has(grant.operationId),
    ),
  });
}

export function filterCapabilities(
  capabilities: readonly ConnectionCapability[],
  query: string,
): ConnectionCapability[] {
  const normalized = query.trim().toLocaleLowerCase();
  if (!normalized) return [...capabilities];
  return capabilities.filter((capability) =>
    [
      capability.name,
      capability.displayName,
      capability.description ?? "",
      capability.capabilityId,
      ...capability.permissionLabels,
    ].some((value) => value.toLocaleLowerCase().includes(normalized)),
  );
}

function normalizeBinding(
  binding: AgentConnectionBinding,
): AgentConnectionBinding {
  return {
    ...binding,
    operationGrants: [
      ...new Set(
        binding.operationGrants
          .map((grant) => grant.operationId.trim())
          .filter(Boolean),
      ),
    ]
      .sort()
      .map((operationId) => ({ operationId })),
  };
}

function capabilityMap(
  capabilities: readonly ConnectionCapability[],
): Map<string, ConnectionCapability> {
  return new Map(
    capabilities.map((capability) => [capability.capabilityId, capability]),
  );
}

function operationFingerprint(capability: ConnectionCapability): string {
  return JSON.stringify({
    kind: capability.kind,
    name: capability.name,
    displayName: capability.displayName,
    description: capability.description ?? null,
    inputSchema: capability.inputSchema,
    annotations: capability.annotations,
    providerMetadata: capability.providerMetadata,
    permissionLabels: [...capability.permissionLabels].sort(),
  });
}
