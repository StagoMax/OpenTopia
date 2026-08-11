import assert from "node:assert/strict";
import test from "node:test";

import type * as FileLinkContextMenuModule from "./fileLinkContextMenu";

const {
  fileLinkClipboardPath,
  fitContextMenuPosition,
}: typeof FileLinkContextMenuModule = await import(
  "./fileLinkContextMenu" + ".ts"
);

test("keeps a file-link menu inside the viewport", () => {
  assert.deepEqual(
    fitContextMenuPosition(
      { x: 990, y: 790 },
      { width: 240, height: 320 },
      { width: 1_000, height: 800 },
      8,
    ),
    { x: 752, y: 472 },
  );
});

test("preserves a context-menu point that already fits", () => {
  assert.deepEqual(
    fitContextMenuPosition(
      { x: 120, y: 160 },
      { width: 240, height: 320 },
      { width: 1_000, height: 800 },
      8,
    ),
    { x: 120, y: 160 },
  );
});

test("copies a normal Windows path without the verbatim prefix", () => {
  assert.equal(
    fileLinkClipboardPath("\\\\?\\J:\\Project\\OpenTopia\\appearance.ts"),
    "J:\\Project\\OpenTopia\\appearance.ts",
  );
  assert.equal(
    fileLinkClipboardPath("\\\\?\\UNC\\server\\share\\appearance.ts"),
    "\\\\server\\share\\appearance.ts",
  );
});
