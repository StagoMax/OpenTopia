import assert from "node:assert/strict";
import test from "node:test";
import {
  composerContentText,
  composerUndoEntries,
  composerVisibleText,
  normalizeComposerContentParts,
  normalizeComposerImageDeletionSnapshot,
  referencedImageIds,
} from "./composerContent.ts";

const imageId = "11111111-1111-4111-8111-111111111111";

test("keeps repeated image references while identifying one unique attachment", () => {
  const parts = normalizeComposerContentParts([
    { type: "text", text: "请按" },
    { type: "image_ref", imageId },
    { type: "text", text: "实现设置页，再参考" },
    { type: "image_ref", imageId },
    { type: "text", text: "调整细节" },
  ]);

  assert.equal(parts.filter((part) => part.type === "image_ref").length, 2);
  assert.deepEqual([...referencedImageIds(parts)], [imageId]);
});

test("builds a readable fallback prompt without exposing internal image IDs", () => {
  const text = composerContentText(
    [
      { type: "text", text: "请按" },
      { type: "image_ref", imageId },
      { type: "text", text: "实现设置页" },
    ],
    [
      {
        id: imageId,
        contentType: "image/png",
        data: [1, 2, 3],
        name: "settings.png",
      },
    ],
  );

  assert.equal(text, "请按[图片：settings.png]实现设置页");
  assert.doesNotMatch(text, /11111111/);
});

test("keeps image filenames out of the visible composer text", () => {
  const text = composerVisibleText([
    { type: "text", text: "请按" },
    { type: "image_ref", imageId },
    { type: "text", text: "实现设置页" },
  ]);

  assert.equal(text, "请按实现设置页");
  assert.doesNotMatch(text, /settings|图片|11111111/);
});

test("splits a multi-character IME commit into one undo entry per character", () => {
  const entries = composerUndoEntries(
    { parts: [], caretOffset: 0 },
    { parts: [{ type: "text", text: "你好" }], caretOffset: 2 },
    true,
  );

  assert.deepEqual(entries, [
    { parts: [], caretOffset: 0 },
    { parts: [{ type: "text", text: "你" }], caretOffset: 1 },
  ]);
});

test("keeps a pasted phrase as one undo operation", () => {
  const entries = composerUndoEntries(
    { parts: [{ type: "text", text: "前" }], caretOffset: 1 },
    { parts: [{ type: "text", text: "前粘贴内容" }], caretOffset: 5 },
    false,
  );

  assert.deepEqual(entries, [
    { parts: [{ type: "text", text: "前" }], caretOffset: 1 },
  ]);
});

test("removes a line break Chromium inserts when an inline image is deleted", () => {
  const snapshot = normalizeComposerImageDeletionSnapshot(
    {
      parts: [
        { type: "text", text: "before" },
        { type: "image_ref", imageId },
        { type: "text", text: "after" },
      ],
      caretOffset: 7,
    },
    {
      parts: [{ type: "text", text: "before\nafter" }],
      caretOffset: 7,
    },
  );

  assert.deepEqual(snapshot, {
    parts: [{ type: "text", text: "beforeafter" }],
    caretOffset: 6,
  });
});

test("preserves intentional line breaks when no image was deleted", () => {
  const after = {
    parts: [{ type: "text" as const, text: "before\nafter" }],
    caretOffset: 7,
  };

  assert.equal(
    normalizeComposerImageDeletionSnapshot(
      {
        parts: [{ type: "text", text: "beforeafter" }],
        caretOffset: 6,
      },
      after,
    ),
    after,
  );
});

test("records inline image insertion in the custom undo history", () => {
  const entries = composerUndoEntries(
    { parts: [{ type: "text", text: "前" }], caretOffset: 1 },
    {
      parts: [
        { type: "text", text: "前" },
        { type: "image_ref", imageId },
      ],
      caretOffset: 2,
    },
    true,
  );

  assert.deepEqual(entries, [
    { parts: [{ type: "text", text: "前" }], caretOffset: 1 },
  ]);
});
