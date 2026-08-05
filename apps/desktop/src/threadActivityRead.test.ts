import assert from "node:assert/strict";
import test from "node:test";

import type * as ThreadActivityReadModule from "./threadActivityRead";

const { isThreadActivityUnread }: typeof ThreadActivityReadModule =
  await import("./threadActivityRead" + ".ts");

test("keeps every task status unread until its task has been opened", () => {
  assert.equal(
    isThreadActivityUnread({}, "thread-a", "2026-08-04T10:00:00.000Z"),
    true,
  );
  assert.equal(
    isThreadActivityUnread(
      { "thread-a": "2026-08-04T10:00:00.000Z" },
      "thread-a",
      "2026-08-04T10:00:00.000Z",
    ),
    false,
  );
});

test("shows a later status change after an earlier one was read", () => {
  assert.equal(
    isThreadActivityUnread(
      { "thread-a": "2026-08-04T10:00:00.000Z" },
      "thread-a",
      "2026-08-04T10:05:00.000Z",
    ),
    true,
  );
});
