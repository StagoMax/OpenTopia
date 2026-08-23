import assert from "node:assert/strict";
import test from "node:test";
import { composerAttachmentReferenceId } from "./composerDom.ts";

test("creates a stable opaque ID for the same dropped attachment", () => {
  const first = composerAttachmentReferenceId(
    "notes.pdf",
    1024,
    "application/pdf",
  );
  const second = composerAttachmentReferenceId(
    "notes.pdf",
    1024,
    "application/pdf",
  );

  assert.equal(first, second);
  assert.match(first, /^att-[0-9a-f]{8}$/);
});

test("keeps attachment IDs distinct when file metadata changes", () => {
  assert.notEqual(
    composerAttachmentReferenceId("notes.pdf", 1024, "application/pdf"),
    composerAttachmentReferenceId("notes.pdf", 2048, "application/pdf"),
  );
});
