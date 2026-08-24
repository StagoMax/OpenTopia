import assert from "node:assert/strict";
import test from "node:test";

import type * as ThreadRecencyModule from "./threadRecency";
import type { Thread } from "./types";

const { promoteThreadByActivity }: typeof ThreadRecencyModule = await import(
  "./threadRecency" + ".ts"
);

function thread(id: string, updatedAt: string): Thread {
  return {
    id,
    title: id,
    workspaceRoot: "J:\\Project\\OpenTopia",
    projectId: "project-1",
    experienceMode: "code",
    modelSelection: null,
    archivedAt: null,
    createdAt: "2026-08-01T00:00:00.000Z",
    updatedAt,
  };
}

test("promotes a thread when local conversation activity starts", () => {
  const first = thread("first", "2026-08-24T08:02:00.000Z");
  const active = thread("active", "2026-08-24T08:00:00.000Z");
  const third = thread("third", "2026-08-24T07:59:00.000Z");

  const result = promoteThreadByActivity(
    [first, active, third],
    active.id,
    "2026-08-24T08:03:00.000Z",
  );

  assert.deepEqual(
    result.map((item) => item.id),
    ["active", "first", "third"],
  );
  assert.equal(result[0].updatedAt, "2026-08-24T08:03:00.000Z");
  assert.strictEqual(result[1], first);
  assert.strictEqual(result[2], third);
});

test("never moves a thread timestamp backwards", () => {
  const active = thread("active", "2026-08-24T08:04:00.000Z");

  const result = promoteThreadByActivity(
    [thread("first", "2026-08-24T08:05:00.000Z"), active],
    active.id,
    "2026-08-24T08:03:00.000Z",
  );

  assert.equal(result[0].id, "active");
  assert.equal(result[0].updatedAt, "2026-08-24T08:04:00.000Z");
});

test("leaves the collection untouched for an unknown thread", () => {
  const threads = [thread("first", "2026-08-24T08:02:00.000Z")];

  assert.strictEqual(
    promoteThreadByActivity(threads, "missing", "2026-08-24T08:03:00.000Z"),
    threads,
  );
});
