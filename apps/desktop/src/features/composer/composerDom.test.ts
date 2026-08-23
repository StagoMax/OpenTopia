import assert from "node:assert/strict";
import test from "node:test";
import { composerTextInsertionValue } from "./composerDom.ts";

test("adds a caret marker only to otherwise invisible trailing line breaks", () => {
  assert.equal(composerTextInsertionValue("\n"), "\n\u200b");
  assert.equal(composerTextInsertionValue("\n2. "), "\n2. ");
  assert.equal(composerTextInsertionValue("\n\u200b1. "), "\n1. ");
});
