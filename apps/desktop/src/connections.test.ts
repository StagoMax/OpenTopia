import assert from "node:assert/strict";
import test from "node:test";
import type { Connection } from "./types";
import type * as ConnectionsModel from "./components/connections/model";

const {
  connectionFormFromConnection,
  connectionInputFromForm,
  connectionStatusLabel,
  connectionUpdateFromInput,
  emptyConnectionForm,
  sortConnections,
  validateConnectionForm,
} = (await import(
  "./components/connections/model" + ".ts"
)) as typeof ConnectionsModel;

function connection(
  id: string,
  name: string,
  status: Connection["status"],
): Connection {
  return {
    schemaVersion: 1,
    id,
    revision: 1,
    integrationDefinitionId: "definition-1",
    name,
    ownerType: "personal",
    environment: "production",
    enabled: true,
    status,
    runtimeBinding: { kind: "mcp_server", serverId: `server-${id}` },
    authContext: {
      account: {},
      grantedScopes: [],
      verification: "not_required",
    },
    createdAt: "2026-08-20T00:00:00Z",
    updatedAt: "2026-08-20T00:00:00Z",
  };
}

test("sortConnections prioritizes operator attention before ready connections", () => {
  const sorted = sortConnections([
    connection("ready", "Ready", "ready"),
    connection("disabled", "Disabled", "disabled"),
    connection("reauth", "Reauth", "reauth_required"),
    connection("degraded", "Degraded", "degraded"),
  ]);
  assert.deepEqual(
    sorted.map((item) => item.id),
    ["reauth", "degraded", "ready", "disabled"],
  );
  assert.equal(connectionStatusLabel(sorted[0].status), "需重新授权");
});

test("validateConnectionForm rejects a runtime already owned by another connection", () => {
  const values = {
    ...emptyConnectionForm([]),
    integrationDefinitionId: "definition-1",
    name: "Sales account",
    environment: "production",
    serverId: "mcp-sales",
  };
  assert.deepEqual(validateConnectionForm(values, new Set(["mcp-sales"])), {
    server: "该 MCP runtime 已绑定其他 Connection",
  });
});

test("connectionInputFromForm normalizes account fields and granted scopes", () => {
  const input = connectionInputFromForm({
    ...emptyConnectionForm([]),
    integrationDefinitionId: "definition-1",
    name: "  CRM production  ",
    environment: " prod ",
    serverId: "mcp-crm",
    tenantName: "  Acme  ",
    credentialRef: "  vault://crm/account  ",
    grantedScopes: "crm.read, deals.write, crm.read",
  });
  assert.equal(input.name, "CRM production");
  assert.equal(input.environment, "prod");
  assert.equal(input.authContext.account.tenantName, "Acme");
  assert.equal(input.authContext.credentialRef, "vault://crm/account");
  assert.deepEqual(input.authContext.grantedScopes, [
    "crm.read",
    "deals.write",
  ]);
});

test("connectionUpdateFromInput omits immutable provider and unchanged auth context", () => {
  const current = connection("crm", "CRM production", "ready");
  current.authContext = {
    credentialRef: "vault://crm/account",
    account: { tenantName: "Acme" },
    grantedScopes: ["crm.read"],
    verification: "verified",
  };
  const input = connectionInputFromForm({
    ...connectionFormFromConnection(current),
    name: "CRM primary",
  });
  assert.deepEqual(connectionUpdateFromInput(current, input), {
    expectedRevision: 1,
    name: "CRM primary",
  });
});
