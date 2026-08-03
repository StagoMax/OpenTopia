import assert from "node:assert/strict";
import test from "node:test";

import type * as ThreadTitleRetryModule from "./threadTitleRetry";

const { threadTitleRetryDelay } = (await import(
  "./threadTitleRetry" + ".ts"
)) as typeof ThreadTitleRetryModule;

test("retries title generation with bounded backoff", () => {
  assert.equal(threadTitleRetryDelay(0), null);
  assert.equal(threadTitleRetryDelay(1), 30_000);
  assert.equal(threadTitleRetryDelay(2), 120_000);
  assert.equal(threadTitleRetryDelay(3), null);
});
