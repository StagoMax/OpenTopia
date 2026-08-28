import assert from "node:assert/strict";
import test from "node:test";
import fixture from "./generated/desktop-http-v1.fixture.json" with { type: "json" };
import { ApiContractError } from "./sseContracts.ts";
import { decodeHttpResponse } from "./httpContracts.ts";

test("decodes a Rust-serialized HTTP fixture", () => {
  assert.deepEqual(decodeHttpResponse("health", fixture), fixture);
});

test("decodes the new Flow case collection contract", () => {
  assert.deepEqual(decodeHttpResponse("listFlowCases", []), []);
});

test("rejects an HTTP response that drifted from its Rust DTO", () => {
  assert.throws(
    () =>
      decodeHttpResponse("health", {
        ...fixture,
        apiVersion: "1",
      }),
    ApiContractError,
  );
});

test("does not apply endpoint contracts to same-named response fields", () => {
  const now = "2026-08-27T00:00:00Z";
  const id = "00000000-0000-4000-8000-000000000001";
  const serverId = "00000000-0000-4000-8000-000000000002";

  assert.doesNotThrow(() =>
    decodeHttpResponse("testConnection", {
      connection: {
        authContext: { verification: "legacy_unverified" },
        createdAt: now,
        enabled: true,
        environment: "local",
        id,
        integrationDefinitionId: id,
        name: "tokenhub-tools",
        ownerType: "personal",
        revision: 1,
        runtimeBinding: { kind: "mcp_server", serverId },
        schemaVersion: 1,
        status: "ready",
        updatedAt: now,
      },
      health: {
        authStatus: "legacy_unverified",
        checkedAt: now,
        message: "ready",
        ok: true,
        runtimeStatus: "ready",
        toolsCount: 1,
      },
    }),
  );
});

test("does not allow an unregistered response contract", () => {
  assert.throws(
    () => decodeHttpResponse("missing" as never, {}),
    /contract is not registered/,
  );
});
