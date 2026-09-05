import type {
  AgentConnectionBinding,
  Connection,
  ConnectionCapability,
  ConnectionCapabilityRevision,
  IntegrationDefinition,
} from "../../types";
import {
  defaultApplicationLanguage,
  interfaceMessage,
  type ApplicationLanguage,
  type InterfaceMessageKey,
} from "../../applicationLanguage.ts";

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
  language: ApplicationLanguage = defaultApplicationLanguage,
): ConnectionGrantEligibility {
  if (definition === null) {
    return {
      selectable: false,
      warning: false,
      reason: message(
        language,
        "flow.connectionGrants.eligibility.definitionMissing",
      ),
    };
  }
  if (definition && !definition.enabled) {
    return {
      selectable: false,
      warning: false,
      reason: message(
        language,
        "flow.connectionGrants.eligibility.definitionDisabled",
      ),
    };
  }
  if (!connection.enabled || connection.status === "disabled") {
    return {
      selectable: false,
      warning: false,
      reason: message(language, "flow.connectionGrants.eligibility.disabled"),
    };
  }
  if (connection.status !== "ready") {
    const reason = {
      configured: message(
        language,
        "flow.connectionGrants.eligibility.configured",
      ),
      degraded: message(language, "flow.connectionGrants.eligibility.degraded"),
      reauth_required: message(
        language,
        "flow.connectionGrants.eligibility.reauth",
      ),
      disabled: message(language, "flow.connectionGrants.eligibility.disabled"),
      ready: null,
    }[connection.status];
    return { selectable: false, warning: false, reason };
  }
  if (connection.activeCapabilityRevision == null) {
    return {
      selectable: false,
      warning: false,
      reason: message(language, "flow.connectionGrants.eligibility.noSnapshot"),
    };
  }
  if (
    connection.authContext.expiresAt != null &&
    Date.parse(connection.authContext.expiresAt) <= nowMs
  ) {
    return {
      selectable: false,
      warning: false,
      reason: message(language, "flow.connectionGrants.eligibility.expired"),
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
        reason: message(
          language,
          "flow.connectionGrants.eligibility.unverified",
        ),
      };
    }
    return {
      selectable: true,
      warning: true,
      reason: message(
        language,
        "flow.connectionGrants.eligibility.legacyWarning",
      ),
    };
  }
  if (
    connection.authContext.verification !== "verified" &&
    connection.authContext.verification !== "not_required"
  ) {
    return {
      selectable: false,
      warning: false,
      reason: message(
        language,
        "flow.connectionGrants.eligibility.accountUnverified",
      ),
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

export function setOperationGrants(
  binding: AgentConnectionBinding,
  operationIds: readonly string[],
  granted: boolean,
): AgentConnectionBinding {
  const current = new Set(
    binding.operationGrants.map((grant) => grant.operationId),
  );
  for (const operationId of operationIds) {
    if (granted) current.add(operationId);
    else current.delete(operationId);
  }
  return normalizeBinding({
    ...binding,
    operationGrants: [...current].map((operationId) => ({ operationId })),
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

function message(
  language: ApplicationLanguage,
  key: InterfaceMessageKey,
): string {
  return interfaceMessage(language, key);
}
