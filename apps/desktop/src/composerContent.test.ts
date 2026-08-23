import assert from "node:assert/strict";
import test from "node:test";
import {
  composerAttachmentReferenceText,
  composerContentText,
  filterComposerAttachmentReferences,
  composerImageDisplayId,
  composerLineBreakText,
  composerOrderedListContinuation,
  composerTextLength,
  composerUndoEntries,
  composerUsesOrderedListIndentation,
  composerVisibleText,
  composerWireContentParts,
  normalizeComposerContentParts,
  normalizeComposerImageDeletionSnapshot,
  referencedAttachmentPaths,
  referencedImageIds,
} from "./composerContent.ts";
import type { InlineMessageContentPart } from "./types/platform";

const imageId = "11111111-1111-4111-8111-111111111111";
const attachmentPath = "C:/Temp/需求说明.pdf";

test("counts composer graphemes without splitting common text into arrays", () => {
  assert.equal(composerTextLength("plain text"), 10);
  assert.equal(composerTextLength("输入框"), 3);
  assert.equal(composerTextLength("👨‍👩‍👧‍👦"), 1);
});

test("filters inline file references by normalized source path, not filename", () => {
  const parts: InlineMessageContentPart[] = [
    { type: "text", text: "请看" },
    {
      type: "attachment_ref",
      path: "C:\\Work\\需求说明.pdf",
      name: "需求说明.pdf",
    },
    {
      type: "attachment_ref",
      path: "C:\\Other\\需求说明.pdf",
      name: "需求说明.pdf",
    },
  ];

  assert.deepEqual(
    filterComposerAttachmentReferences(parts, ["c:/work/需求说明.pdf"]),
    [
      { type: "text", text: "请看" },
      {
        type: "attachment_ref",
        path: "C:\\Work\\需求说明.pdf",
        name: "需求说明.pdf",
      },
    ],
  );
});

test("uses a list-aware line break for Shift+Enter", () => {
  const snapshot = {
    parts: [{ type: "text" as const, text: "1. 第一项" }],
    caretOffset: composerTextLength("1. 第一项"),
  };
  assert.equal(composerLineBreakText(snapshot, false), "\n");
  assert.equal(composerLineBreakText(snapshot, true), "\n2. ");
});

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

test("builds a readable fallback prompt with the short image ID", () => {
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

  assert.equal(text, "请按[图片：11111111]实现设置页");
  assert.doesNotMatch(text, /11111111-1111/);
});

test("uses a compact stable image ID in visible attachment labels", () => {
  assert.equal(composerImageDisplayId(imageId), "11111111");
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

test("keeps plain-text undo offsets correct when IME inserts before a suffix", () => {
  const entries = composerUndoEntries(
    { parts: [{ type: "text", text: "前后" }], caretOffset: 1 },
    { parts: [{ type: "text", text: "前你好后" }], caretOffset: 3 },
    true,
  );

  assert.deepEqual(entries, [
    { parts: [{ type: "text", text: "前后" }], caretOffset: 1 },
    { parts: [{ type: "text", text: "前你后" }], caretOffset: 2 },
  ]);
});

test("records a plain-text deletion as one undo operation", () => {
  const before = {
    parts: [{ type: "text" as const, text: "需要删除" }],
    caretOffset: 4,
  };
  const entries = composerUndoEntries(
    before,
    { parts: [{ type: "text", text: "需要删" }], caretOffset: 3 },
    false,
  );

  assert.deepEqual(entries, [before]);
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

test("renders an inline attachment reference as its filename only", () => {
  assert.equal(
    composerAttachmentReferenceText("需求说明.pdf"),
    "[需求说明.pdf]",
  );
});

test("keeps attachment filenames in visible text while excluding images", () => {
  const text = composerVisibleText([
    { type: "text", text: "请按" },
    { type: "attachment_ref", path: attachmentPath, name: "需求说明.pdf" },
    { type: "image_ref", imageId },
    { type: "text", text: "实现" },
  ]);

  assert.equal(text, "请按[需求说明.pdf]实现");
});

test("collects the distinct paths referenced by inline attachments", () => {
  assert.deepEqual(
    [
      ...referencedAttachmentPaths([
        { type: "text", text: "a" },
        {
          type: "attachment_ref",
          path: attachmentPath,
          name: "需求说明.pdf",
        },
        {
          type: "attachment_ref",
          path: attachmentPath,
          name: "需求说明.pdf",
        },
      ]),
    ],
    [attachmentPath],
  );
});

test("flattens attachment references to text for the wire and keeps images", () => {
  const parts: InlineMessageContentPart[] = [
    { type: "text", text: "请按" },
    { type: "attachment_ref", path: attachmentPath, name: "需求说明.pdf" },
    { type: "image_ref", imageId },
  ];

  assert.deepEqual(composerWireContentParts(parts), [
    { type: "text", text: "请按" },
    { type: "text", text: "[需求说明.pdf]" },
    { type: "image_ref", imageId },
  ]);
});

test("builds readable fallback text for mixed inline references", () => {
  const text = composerContentText(
    [
      { type: "text", text: "按" },
      { type: "attachment_ref", path: attachmentPath, name: "需求说明.pdf" },
      { type: "image_ref", imageId },
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

  assert.equal(text, "按[需求说明.pdf][图片：11111111]");
});

test("removes a line break Chromium inserts when an attachment reference is deleted", () => {
  const snapshot = normalizeComposerImageDeletionSnapshot(
    {
      parts: [
        { type: "text", text: "before" },
        { type: "attachment_ref", path: attachmentPath, name: "需求说明.pdf" },
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

test("records inline attachment insertion in the custom undo history", () => {
  const entries = composerUndoEntries(
    { parts: [{ type: "text", text: "前" }], caretOffset: 1 },
    {
      parts: [
        { type: "text", text: "前" },
        {
          type: "attachment_ref",
          path: attachmentPath,
          name: "需求说明.pdf",
        },
      ],
      caretOffset: 2,
    },
    true,
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

test("does not continue numbering when the marker is not the line's first content", () => {
  for (const text of ["说明 1. 第一项", "- 1. 第一项", "前缀\t1. 第一项"]) {
    assert.equal(
      composerOrderedListContinuation({
        parts: [{ type: "text", text }],
        caretOffset: composerTextLength(text),
      }),
      null,
    );
  }
});

test("accepts only CommonMark indentation before an ordered-list marker", () => {
  const valid = "   1. 第一项";
  const codeIndented = "    1. 第一项";
  const tabIndented = "\t1. 第一项";
  assert.equal(
    composerOrderedListContinuation({
      parts: [{ type: "text", text: valid }],
      caretOffset: composerTextLength(valid),
    }),
    "\n   2. ",
  );
  assert.equal(
    composerOrderedListContinuation({
      parts: [{ type: "text", text: codeIndented }],
      caretOffset: composerTextLength(codeIndented),
    }),
    null,
  );
  assert.equal(
    composerOrderedListContinuation({
      parts: [{ type: "text", text: tabIndented }],
      caretOffset: composerTextLength(tabIndented),
    }),
    null,
  );
});

test("continues an ordered list on the line after an image reference", () => {
  const text = "\n1. ";
  assert.equal(
    composerOrderedListContinuation({
      parts: [
        { type: "image_ref", imageId },
        { type: "text", text },
      ],
      caretOffset: 1 + composerTextLength(text),
    }),
    "\n2. ",
  );
});

test("does not treat an image and list marker on the same line as a list", () => {
  const text = "1. 第一项";
  assert.equal(
    composerOrderedListContinuation({
      parts: [
        { type: "image_ref", imageId },
        { type: "text", text },
      ],
      caretOffset: 1 + composerTextLength(text),
    }),
    null,
  );
});

test("continues an ordered list when the composer inserts a line break", () => {
  const text = "1. ";
  assert.equal(
    composerOrderedListContinuation({
      parts: [{ type: "text", text }],
      caretOffset: composerTextLength(text),
    }),
    "\n2. ",
  );
});

test("continues the current ordered-list line and preserves indentation", () => {
  const text = "说明\n  9. 第九项";
  assert.equal(
    composerOrderedListContinuation({
      parts: [{ type: "text", text }],
      caretOffset: composerTextLength(text),
    }),
    "\n  10. ",
  );
});

test("does not invent numbering for ordinary lines or image content", () => {
  assert.equal(
    composerOrderedListContinuation({
      parts: [{ type: "text", text: "普通文本" }],
      caretOffset: 4,
    }),
    null,
  );
  assert.equal(
    composerOrderedListContinuation({
      parts: [{ type: "image_ref", imageId }],
      caretOffset: 1,
    }),
    null,
  );
});

test("uses rendered Markdown indentation after an ordered-list marker", () => {
  assert.equal(composerUsesOrderedListIndentation("1. "), true);
  assert.equal(composerUsesOrderedListIndentation("\n1. "), true);
  assert.equal(
    composerUsesOrderedListIndentation("  1. 第一项\n  2. 第二项"),
    true,
  );
  assert.equal(composerUsesOrderedListIndentation("1.还没有空格"), false);
  assert.equal(composerUsesOrderedListIndentation("说明 1. 第一项"), false);
  assert.equal(composerUsesOrderedListIndentation("说明\n1. 第一项"), true);
  assert.equal(composerUsesOrderedListIndentation("    1. 这是代码块"), false);
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
