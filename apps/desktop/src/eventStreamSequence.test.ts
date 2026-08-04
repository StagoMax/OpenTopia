import assert from "node:assert/strict";
import test from "node:test";

import type * as EventStreamSequenceModule from "./eventStreamSequence";

const { shouldRecoverEventSequenceGap } = (await import(
  "./eventStreamSequence" + ".ts"
)) as typeof EventStreamSequenceModule;

test("projected conversation streams accept filtered sequence gaps", () => {
  assert.equal(shouldRecoverEventSequenceGap(8, 10, "projected"), false);
});

test("contiguous streams still recover real sequence gaps", () => {
  assert.equal(shouldRecoverEventSequenceGap(8, 10, "contiguous"), true);
  assert.equal(shouldRecoverEventSequenceGap(8, 9, "contiguous"), false);
});
