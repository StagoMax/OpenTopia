import assert from "node:assert/strict";
import test from "node:test";
import type { ApiClient } from "../../api/client.ts";
import type { Connection } from "../../types.ts";
import { ConnectionsStore } from "./store.ts";

function connection(id: string, name: string): Connection {
  return {
    schemaVersion: 1,
    id,
    revision: 1,
    integrationDefinitionId: "mcp",
    name,
    ownerType: "personal",
    environment: "production",
    enabled: true,
    status: "ready",
    runtimeBinding: { kind: "mcp_server", serverId: `server-${id}` },
    authContext: {
      account: {},
      grantedScopes: [],
      verification: "not_required",
    },
    createdAt: "2026-08-30T00:00:00.000Z",
    updatedAt: "2026-08-30T00:00:00.000Z",
  };
}

test("reveal selects a requested connection while the collection loads", async () => {
  const client = {
    listIntegrationDefinitions: async () => [],
    listConnections: async () => [
      connection("first", "First"),
      connection("target", "Target"),
    ],
    listMcpServers: async () => [],
    listConnectionCapabilityRevisions: async () => [],
  } as unknown as ApiClient;
  const store = new ConnectionsStore(client);

  store.reveal("target");
  await store.load();

  assert.equal(store.getSnapshot().selectedConnectionId, "target");
});
