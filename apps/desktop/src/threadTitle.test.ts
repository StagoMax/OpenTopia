import assert from "node:assert/strict";
import test from "node:test";

import type * as ThreadTitleModule from "./threadTitle";

const { threadTitleFromPrompt, threadTitleNeedsSummary } = (await import(
  "./threadTitle" + ".ts"
)) as typeof ThreadTitleModule;

test("keeps short prompts and truncates long local titles", () => {
  assert.equal(threadTitleFromPrompt("  修复登录问题  "), "修复登录问题");
  const longPrompt = "一".repeat(51);
  assert.equal(threadTitleNeedsSummary(longPrompt), true);
  assert.equal(Array.from(threadTitleFromPrompt(longPrompt)).length, 50);
  assert.equal(threadTitleFromPrompt(longPrompt).endsWith("…"), true);
});
