import assert from "node:assert/strict";
import test from "node:test";
import { threadTitleScrollDurationMs } from "./threadTitleScroll.ts";

test("does not animate a title without measurable overflow", () => {
  assert.equal(threadTitleScrollDurationMs(0), 0);
  assert.equal(threadTitleScrollDurationMs(-10), 0);
  assert.equal(threadTitleScrollDurationMs(Number.NaN), 0);
});

test("keeps a short minimum while avoiding a long fixed wait", () => {
  assert.equal(threadTitleScrollDurationMs(8), 600);
  assert.equal(threadTitleScrollDurationMs(24), 600);
  assert.equal(threadTitleScrollDurationMs(48), 600);
  assert.equal(threadTitleScrollDurationMs(64), 800);
});

test("uses a consistent pixel speed for every overflowing title", () => {
  assert.equal(threadTitleScrollDurationMs(80), 1_000);
  assert.equal(threadTitleScrollDurationMs(160), 2_000);
  assert.equal(threadTitleScrollDurationMs(320), 4_000);
});
