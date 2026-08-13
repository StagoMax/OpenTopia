import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

test("OpenTopia adapter emits one spawn event per queued subagent", async () => {
  const source = await readFile(
    new URL("../adapters/opentopia-http.mjs", import.meta.url),
    "utf8",
  );

  assert.match(source, /if \(status === "queued"\)/);
  assert.match(source, /type: "subagent\.running"/);
  assert.match(source, /"timed_out"/);
  assert.match(source, /status === "timed_out" \? "subagent\.failed"/);
  assert.doesNotMatch(source, /\["queued", "running"\]\.includes\(status\)/);
});
