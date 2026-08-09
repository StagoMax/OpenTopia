import assert from "node:assert/strict";
import test from "node:test";

import { auditAccess, summarizeAudit, validateGrants } from "../src/access.js";

const grants = [
  { id: "g1", user: "alice", resource: "repo", role: "viewer", source: "group", expiresAt: null },
  { id: "g2", user: "alice", resource: "repo", role: "editor", source: "direct", expiresAt: null },
  { id: "g3", user: "bob", resource: "db", role: "admin", source: "group", expiresAt: "2025-01-01T00:00:00Z" },
];

test("validates and canonicalizes grants", () => {
  const result = validateGrants(grants.slice().reverse());
  assert.deepEqual(result.map((grant) => grant.id), ["g1", "g2", "g3"]);
  assert.equal(result[2].expiresAt, "2025-01-01T00:00:00.000Z");
});

test("selects effective grants and classifies the rest", () => {
  const audit = auditAccess(grants, "2026-01-01T00:00:00Z");
  assert.deepEqual(audit.effective, [
    { user: "alice", resource: "repo", role: "editor", source: "direct", grantId: "g2" },
  ]);
  assert.deepEqual(audit.expired, [
    { grantId: "g3", user: "bob", resource: "db", expiredAt: "2025-01-01T00:00:00.000Z" },
  ]);
  assert.deepEqual(audit.shadowed, [
    { grantId: "g1", effectiveGrantId: "g2", user: "alice", resource: "repo" },
  ]);
});

test("summarizes an audit", () => {
  assert.deepEqual(summarizeAudit(auditAccess(grants, "2024-01-01T00:00:00Z")), {
    grants: 3,
    effective: 2,
    expired: 0,
    shadowed: 1,
    adminAccess: 1,
  });
});
