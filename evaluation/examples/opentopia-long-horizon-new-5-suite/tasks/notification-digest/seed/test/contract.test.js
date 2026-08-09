import assert from "node:assert/strict";
import test from "node:test";

import { buildDigests, summarizeDigests, validateEvents } from "../src/digest.js";

const events = [
  { id: "a", recipient: "zoe", category: "build", severity: "warning", createdAt: "2026-02-01T00:00:02Z", read: false },
  { id: "b", recipient: "amy", category: "security", severity: "critical", createdAt: "2026-02-01T00:00:01Z", read: false },
  { id: "c", recipient: "amy", category: "news", severity: "info", createdAt: "2026-01-01T00:00:00Z", read: false },
  { id: "d", recipient: "amy", category: "build", severity: "warning", createdAt: "2026-02-01T00:00:03Z", read: true },
];

test("validates and canonicalizes events", () => {
  const result = validateEvents(events.slice().reverse());
  assert.deepEqual(result.map((event) => event.id), ["a", "b", "c", "d"]);
  assert.equal(result[0].createdAt, "2026-02-01T00:00:02.000Z");
});

test("filters, groups, and orders digest items", () => {
  const result = buildDigests(events, "2026-02-01T00:00:00Z");
  assert.deepEqual(result.digests.map((digest) => digest.recipient), ["amy", "zoe"]);
  assert.deepEqual(result.digests[0], {
    recipient: "amy",
    critical: 1,
    warning: 0,
    info: 0,
    items: [{ id: "b", category: "security", severity: "critical", createdAt: "2026-02-01T00:00:01.000Z" }],
  });
});

test("summarizes generated digests", () => {
  assert.deepEqual(summarizeDigests(buildDigests(events, "2026-02-01T00:00:00Z")), {
    recipients: 2,
    notifications: 2,
    critical: 1,
    warning: 1,
    info: 0,
  });
});
