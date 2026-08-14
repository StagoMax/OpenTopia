import assert from "node:assert/strict";
import test from "node:test";

import { hasFileDragPayload } from "./fileDrop.ts";

test("recognizes operating-system file drags", () => {
  assert.equal(hasFileDragPayload(["text/plain", "Files"]), true);
});

test("leaves text and in-app drags alone", () => {
  assert.equal(hasFileDragPayload([]), false);
  assert.equal(hasFileDragPayload(["text/plain", "text/uri-list"]), false);
});
