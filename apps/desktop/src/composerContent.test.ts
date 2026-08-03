import assert from "node:assert/strict";
import test from "node:test";
import {
  composerContentText,
  normalizeComposerContentParts,
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
