import assert from "node:assert/strict";
import test from "node:test";

import type * as ThreadTitleModule from "./threadTitle";

const { conversationHeaderTitle, threadTitleFromPrompt } = (await import(
  "./threadTitle" + ".ts"
)) as typeof ThreadTitleModule;

test("keeps prompts up to 100 Unicode characters", () => {
  assert.equal(threadTitleFromPrompt("  修复登录问题  "), "修复登录问题");
  assert.equal(Array.from(threadTitleFromPrompt("一".repeat(100))).length, 100);
});

test("truncates longer prompts locally with an ellipsis", () => {
  const longPrompt = "一".repeat(101);
  assert.equal(Array.from(threadTitleFromPrompt(longPrompt)).length, 100);
  assert.equal(threadTitleFromPrompt(longPrompt).endsWith("…"), true);
});

test("normalizes whitespace before truncating", () => {
  const prompt = `${"一".repeat(98)}\n\n最后一句`;
  const title = threadTitleFromPrompt(prompt);
  assert.equal(Array.from(title).length, 100);
  assert.equal(title, `${"一".repeat(98)} …`);
});

test("truncates the conversation header title to 50 Unicode characters", () => {
  const title = conversationHeaderTitle("界".repeat(51));
  assert.equal(Array.from(title).length, 50);
  assert.equal(title, `${"界".repeat(49)}…`);
  assert.equal(conversationHeaderTitle("界".repeat(50)), "界".repeat(50));
});
