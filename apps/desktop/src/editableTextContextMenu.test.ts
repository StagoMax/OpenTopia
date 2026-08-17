import assert from "node:assert/strict";
import test from "node:test";

import type * as EditableTextContextMenuModule from "./editableTextContextMenu";

const { editableTextMenuAvailability } = (await import(
  "./editableTextContextMenu" + ".ts"
)) as typeof EditableTextContextMenuModule;

test("enables all edit actions for selected writable text", () => {
  assert.deepEqual(
    editableTextMenuAvailability({
      readOnly: false,
      selectionLength: 4,
      textLength: 12,
    }),
    {
      canCopy: true,
      canCut: true,
      canPaste: true,
      canSelectAll: true,
    },
  );
});

test("keeps only non-mutating actions for readonly text", () => {
  assert.deepEqual(
    editableTextMenuAvailability({
      readOnly: true,
      selectionLength: 4,
      textLength: 12,
    }),
    {
      canCopy: true,
      canCut: false,
      canPaste: false,
      canSelectAll: true,
    },
  );
});

test("allows paste into an empty writable field", () => {
  assert.deepEqual(
    editableTextMenuAvailability({
      readOnly: false,
      selectionLength: 0,
      textLength: 0,
    }),
    {
      canCopy: false,
      canCut: false,
      canPaste: true,
      canSelectAll: false,
    },
  );
});
