import assert from "node:assert/strict";
import test from "node:test";

import { diffCacheManifest, summarizeDiff, validateEntries } from "../src/cache.js";

const aHash = "A".repeat(64);
const bHash = "b".repeat(64);
const cHash = "c".repeat(64);

test("validates, normalizes, and sorts entries", () => {
  assert.deepEqual(validateEntries([
    { path: "z/file", size: 2, sha256: bHash },
    { path: "a/file", size: 1, sha256: aHash },
  ]), [
    { path: "a/file", size: 1, sha256: "a".repeat(64) },
    { path: "z/file", size: 2, sha256: bHash },
  ]);
});

test("classifies manifest differences", () => {
  const diff = diffCacheManifest([
    { path: "same", size: 1, sha256: aHash },
    { path: "gone", size: 2, sha256: bHash },
    { path: "changed", size: 3, sha256: cHash },
  ], [
    { path: "same", size: 1, sha256: aHash },
    { path: "new", size: 4, sha256: bHash },
    { path: "changed", size: 5, sha256: aHash },
  ]);
  assert.deepEqual(diff.missing.map((entry) => entry.path), ["gone"]);
  assert.deepEqual(diff.unexpected.map((entry) => entry.path), ["new"]);
  assert.deepEqual(diff.changed[0].reasons, ["size", "sha256"]);
  assert.deepEqual(diff.unchanged.map((entry) => entry.path), ["same"]);
});

test("summarizes expected and observed counts", () => {
  const diff = diffCacheManifest([
    { path: "same", size: 1, sha256: aHash },
    { path: "gone", size: 2, sha256: bHash },
  ], [{ path: "same", size: 1, sha256: aHash }]);
  assert.deepEqual(summarizeDiff(diff), {
    expected: 2,
    observed: 1,
    missing: 1,
    unexpected: 0,
    changed: 0,
    unchanged: 1,
    valid: false,
  });
});
