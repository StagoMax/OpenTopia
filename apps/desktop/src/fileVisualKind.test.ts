import assert from "node:assert/strict";
import test from "node:test";

import type * as FileVisualKindModule from "./fileVisualKind";

const { fileVisualKind }: typeof FileVisualKindModule = await import(
  "./fileVisualKind" + ".ts"
);

test("gives PDF and Word attachments distinct visual kinds", () => {
  assert.equal(fileVisualKind("invoice.PDF"), "pdf");
  assert.equal(fileVisualKind("guide.docx"), "word");
  assert.equal(fileVisualKind("legacy.doc"), "word");
});

test("classifies common attachment families by extension", () => {
  assert.equal(fileVisualKind("photo.webp"), "image");
  assert.equal(fileVisualKind("report.xlsx"), "spreadsheet");
  assert.equal(fileVisualKind("pitch.pptx"), "presentation");
  assert.equal(fileVisualKind("recording.mp3"), "audio");
  assert.equal(fileVisualKind("demo.mp4"), "video");
  assert.equal(fileVisualKind("bundle.zip"), "archive");
  assert.equal(fileVisualKind("payload.json"), "data");
  assert.equal(fileVisualKind("component.tsx"), "code");
  assert.equal(fileVisualKind("notes.txt"), "text");
});

test("uses MIME type when a filename has no useful extension", () => {
  assert.equal(fileVisualKind("download", "application/pdf"), "pdf");
  assert.equal(
    fileVisualKind(
      "download",
      "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    ),
    "word",
  );
  assert.equal(
    fileVisualKind("download", "image/png; charset=binary"),
    "image",
  );
});

test("prefers a known extension over a generic or conflicting MIME type", () => {
  assert.equal(
    fileVisualKind("guide.docx", "application/octet-stream"),
    "word",
  );
  assert.equal(fileVisualKind("invoice.pdf", "text/plain"), "pdf");
});

test("falls back safely for unknown attachment formats", () => {
  assert.equal(fileVisualKind("README"), "generic");
  assert.equal(fileVisualKind("payload.unknown"), "generic");
});
