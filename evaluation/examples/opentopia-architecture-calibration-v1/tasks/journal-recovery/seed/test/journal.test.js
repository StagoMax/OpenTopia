import assert from "node:assert/strict";
import crypto from "node:crypto";
import test from "node:test";
import { parseJournal, recover } from "../src/journal.js";

const frame = (sequence, operation) => {
  const checksum = crypto.createHash("sha256").update(JSON.stringify({ sequence, operation })).digest("hex");
  return JSON.stringify({ sequence, operation, checksum });
};

test("parses and replays frames", () => {
  const frames = parseJournal(`${frame(2, { type: "put", key: "b", value: 2 })}\n`);
  assert.deepEqual(recover({ sequence: 1, records: { a: 1 } }, frames), { sequence: 2, records: { a: 1, b: 2 } });
});

test("ignores a final truncated line", () => {
  assert.equal(parseJournal(`${frame(1, { type: "delete", key: "x" })}\n{"sequence":`).length, 1);
});
