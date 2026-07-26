import assert from "node:assert/strict";
import test from "node:test";

import type * as ClipboardTextModule from "./clipboardText";

const clipboardText: typeof ClipboardTextModule = await import(
  "./clipboardText" + ".ts"
);

const { normalizeCopiedText } = clipboardText;

test("drops the blank lines a block boundary appends to a copied message", () => {
  assert.equal(normalizeCopiedText("修复完成。\n\n"), "修复完成。");
  assert.equal(normalizeCopiedText("修复完成。\r\n\r\n"), "修复完成。");
  assert.equal(normalizeCopiedText("修复完成。\n \n\t\n"), "修复完成。");
  assert.equal(normalizeCopiedText("修复完成。   "), "修复完成。");
});

test("drops blank lines a selection picks up before the first line", () => {
  assert.equal(normalizeCopiedText("\n\n修复完成。"), "修复完成。");
  assert.equal(normalizeCopiedText("\n  \n修复完成。\n\n"), "修复完成。");
});

test("keeps the interior of a multi-paragraph copy intact", () => {
  assert.equal(normalizeCopiedText("第一段\n\n第二段\n\n"), "第一段\n\n第二段");
  assert.equal(
    normalizeCopiedText("说明：\n\n```\ncode  \n\n  more\n```\n\n"),
    "说明：\n\n```\ncode  \n\n  more\n```",
  );
});

test("preserves the indentation of the first copied line", () => {
  assert.equal(normalizeCopiedText("\n    indented"), "    indented");
});

test("leaves text without edge blank lines untouched", () => {
  assert.equal(normalizeCopiedText("单行"), "单行");
  assert.equal(normalizeCopiedText(""), "");
  assert.equal(normalizeCopiedText("\n\n"), "");
});
