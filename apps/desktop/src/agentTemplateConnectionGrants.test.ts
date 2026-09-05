import assert from "node:assert/strict";
import test from "node:test";
import type {
  AgentConnectionBinding,
  Connection,
  ConnectionCapability,
  ConnectionCapabilityRevision,
  IntegrationDefinition,
} from "./types";
import type * as GrantModel from "./components/agentTemplateConnectionGrants/model";

const {
  bindingFreshness,
  connectionGrantEligibility,
  filterCapabilities,
  hasLegacyMcpProjection,
  normalizeConnectionBindings,
  rebaseBinding,
  replaceConnectionBinding,
  setOperationGrants,
  toggleOperationGrant,
} = (await import(
  "./components/agentTemplateConnectionGrants/model" + ".ts"
)) as typeof GrantModel;

function connection(
  status: Connection["status"] = "ready",
  verification: Connection["authContext"]["verification"] = "verified",
): Connection {
  return {
    schemaVersion: 1,
    id: "connection-1",
    revision: 1,
    integrationDefinitionId: "definition-1",
    name: "CRM production",
    ownerType: "personal",
    environment: "production",
    enabled: status !== "disabled",
    status,
    runtimeBinding: { kind: "mcp_server", serverId: "server-1" },
    authContext: {
      account: { displayName: "Ada" },
      grantedScopes: [],
      verification,
    },
    activeCapabilityRevision: 2,
    createdAt: "2026-08-20T00:00:00Z",
    updatedAt: "2026-08-20T00:00:00Z",
  };
}

function definition(
  key = "legacy-mcp-crm",
  enabled = true,
): IntegrationDefinition {
  return {
    schemaVersion: 1,
    id: "definition-1",
    revision: 1,
    key,
    name: "CRM",
    kind: "mcp",
    authScheme: "external",
    capabilityDiscovery: "mcp_tools_list",
    enabled,
    createdAt: "2026-08-20T00:00:00Z",
    updatedAt: "2026-08-20T00:00:00Z",
  };
}

function operation(
  capabilityId: string,
  description = "Read customers",
): ConnectionCapability {
  const name = capabilityId.split(":").at(-1) ?? capabilityId;
  return {
    capabilityId,
    kind: "tool",
    name,
    displayName: name,
    description,
    inputSchema: { type: "object" },
    annotations: { readOnlyHint: true },
    providerMetadata: {
      serverId: "server-1",
      publicName: `crm__${name}`,
      toolName: name,
    },
    permissionLabels: ["read"],
  };
}

function revision(
  revisionNumber: number,
  capabilities: ConnectionCapability[],
): ConnectionCapabilityRevision {
  return {
    schemaVersion: 1,
    id: `revision-${revisionNumber}`,
    connectionId: "connection-1",
    revision: revisionNumber,
    source: "mcp_tools_list",
    contentHash: `hash-${revisionNumber}`,
    discoveryCoverage: {
      tools: "supported",
      resources: "unsupported",
      prompts: "unsupported",
    },
    capabilities,
    discoveredAt: "2026-08-20T00:00:00Z",
  };
}

const readId = "connection:connection-1:tool:customers.read";
const writeId = "connection:connection-1:tool:customers.write";

function binding(): AgentConnectionBinding {
  return {
    connectionId: "connection-1",
    capabilityRevision: 1,
    operationGrants: [{ operationId: readId }],
  };
}

test("grant eligibility requires ready runtime and verified authentication", () => {
  assert.equal(connectionGrantEligibility(connection()).selectable, true);
  assert.deepEqual(
    connectionGrantEligibility(
      connection("ready", "legacy_unverified"),
      definition(),
    ),
    {
      selectable: true,
      warning: true,
      reason: "旧版凭据未验证；发布前请确认账号与权限范围",
    },
  );
  assert.equal(
    connectionGrantEligibility(connection("degraded")).selectable,
    false,
  );
  assert.equal(
    connectionGrantEligibility(connection("ready", "unverified")).selectable,
    false,
  );
  assert.equal(
    connectionGrantEligibility(connection(), definition("crm", false))
      .selectable,
    false,
  );
  assert.equal(
    connectionGrantEligibility(
      connection("ready", "legacy_unverified"),
      definition("oauth-crm"),
    ).selectable,
    false,
  );
  const credentialBackedLegacy = connection("ready", "legacy_unverified");
  credentialBackedLegacy.authContext.credentialRef = "vault://crm/account";
  assert.equal(
    connectionGrantEligibility(credentialBackedLegacy, definition()).selectable,
    false,
  );
  const expired = connection();
  expired.authContext.expiresAt = "2026-08-19T00:00:00Z";
  assert.deepEqual(
    connectionGrantEligibility(
      expired,
      definition(),
      Date.parse("2026-08-20T00:00:00Z"),
    ),
    {
      selectable: false,
      warning: false,
      reason: "连接登录已过期，请先重新授权",
    },
  );
  assert.equal(
    connectionGrantEligibility(
      connection("degraded"),
      definition(),
      Date.now(),
      "en-US",
    ).reason,
    "The connection is degraded. Fix its health check first",
  );
});

test("legacy MCP projection includes allow-all even without explicit server ids", () => {
  assert.equal(hasLegacyMcpProjection(false, []), false);
  assert.equal(hasLegacyMcpProjection(false, ["server-1"]), true);
  assert.equal(hasLegacyMcpProjection(true, []), true);
});

test("legacy template responses normalize an omitted connectionBindings field", () => {
  assert.deepEqual(normalizeConnectionBindings(undefined), []);
});

test("freshness ignores newly discovered operations but detects granted changes", () => {
  const pinned = revision(1, [operation(readId)]);
  const addedOnly = revision(2, [operation(readId), operation(writeId)]);
  assert.deepEqual(bindingFreshness(binding(), pinned, addedOnly), {
    state: "current",
    changedOperationIds: [],
    removedOperationIds: [],
  });

  const descriptorChanged = revision(2, [
    operation(readId, "Read customers and secrets"),
  ]);
  assert.deepEqual(bindingFreshness(binding(), pinned, descriptorChanged), {
    state: "stale",
    changedOperationIds: [readId],
    removedOperationIds: [],
  });
  assert.deepEqual(bindingFreshness(binding(), pinned, revision(2, [])), {
    state: "stale",
    changedOperationIds: [],
    removedOperationIds: [readId],
  });
});

test("operation toggles normalize duplicates and rebasing drops removed grants", () => {
  const duplicate = {
    ...binding(),
    operationGrants: [
      { operationId: readId },
      { operationId: readId },
      { operationId: writeId },
    ],
  };
  const withoutRead = toggleOperationGrant(duplicate, readId);
  assert.deepEqual(withoutRead.operationGrants, [{ operationId: writeId }]);

  const rebased = rebaseBinding(duplicate, revision(2, [operation(readId)]));
  assert.equal(rebased.capabilityRevision, 2);
  assert.deepEqual(rebased.operationGrants, [{ operationId: readId }]);
});

test("bulk operation grants preserve selections outside the requested set", () => {
  const withWrite = setOperationGrants(binding(), [writeId], true);
  assert.deepEqual(withWrite.operationGrants, [
    { operationId: readId },
    { operationId: writeId },
  ]);

  const withoutWrite = setOperationGrants(withWrite, [writeId], false);
  assert.deepEqual(withoutWrite.operationGrants, [{ operationId: readId }]);
});

test("replaceConnectionBinding maintains one binding per connection", () => {
  const replaced = replaceConnectionBinding(
    [
      binding(),
      {
        connectionId: "connection-2",
        capabilityRevision: 1,
        operationGrants: [{ operationId: "operation-2" }],
      },
    ],
    {
      ...binding(),
      capabilityRevision: 2,
      operationGrants: [{ operationId: writeId }],
    },
  );
  assert.equal(replaced.length, 2);
  assert.equal(replaced.at(-1)?.capabilityRevision, 2);
});

test("capability search includes permission labels and stable operation id", () => {
  const capabilities = [operation(readId), operation(writeId)];
  capabilities[1].permissionLabels = ["destructive"];
  assert.deepEqual(
    filterCapabilities(capabilities, "destructive").map(
      (item) => item.capabilityId,
    ),
    [writeId],
  );
  assert.deepEqual(
    filterCapabilities(capabilities, "customers.read").map(
      (item) => item.capabilityId,
    ),
    [readId],
  );
});
