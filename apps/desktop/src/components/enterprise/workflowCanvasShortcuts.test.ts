import assert from "node:assert/strict";
import test from "node:test";
import { workflowCanvasCommand } from "./workflowCanvasShortcuts.ts";

function key(
  input: Partial<
    Pick<
      KeyboardEvent,
      "altKey" | "code" | "ctrlKey" | "key" | "metaKey" | "shiftKey"
    >
  >,
) {
  return {
    altKey: false,
    code: "",
    ctrlKey: false,
    key: "",
    metaKey: false,
    shiftKey: false,
    ...input,
  };
}

const editable = { disabled: false, readOnly: false };

test("maps canvas tools and viewport controls without modifiers", () => {
  assert.equal(
    workflowCanvasCommand(key({ key: "v" }), editable),
    "selectTool",
  );
  assert.equal(workflowCanvasCommand(key({ key: "H" }), editable), "panTool");
  assert.equal(workflowCanvasCommand(key({ key: "f" }), editable), "fitView");
  assert.equal(
    workflowCanvasCommand(
      key({ code: "Digit1", key: "!", shiftKey: true }),
      editable,
    ),
    "fitView",
  );
  assert.equal(workflowCanvasCommand(key({ key: "+" }), editable), "zoomIn");
  assert.equal(workflowCanvasCommand(key({ key: "=" }), editable), "zoomIn");
  assert.equal(workflowCanvasCommand(key({ key: "-" }), editable), "zoomOut");
});

test("maps edit commands and supports Windows and macOS modifiers", () => {
  assert.equal(
    workflowCanvasCommand(key({ key: "n" }), editable),
    "openNodePicker",
  );
  assert.equal(
    workflowCanvasCommand(key({ ctrlKey: true, key: "z" }), editable),
    "undo",
  );
  assert.equal(
    workflowCanvasCommand(
      key({ key: "z", metaKey: true, shiftKey: true }),
      editable,
    ),
    "redo",
  );
  assert.equal(
    workflowCanvasCommand(key({ ctrlKey: true, key: "y" }), editable),
    "redo",
  );
  assert.equal(
    workflowCanvasCommand(key({ key: "Delete" }), editable),
    "deleteSelection",
  );
});

test("keeps navigation shortcuts in read-only mode and blocks mutations", () => {
  const readOnly = { disabled: false, readOnly: true };
  assert.equal(
    workflowCanvasCommand(key({ key: "v" }), readOnly),
    "selectTool",
  );
  assert.equal(workflowCanvasCommand(key({ key: "f" }), readOnly), "fitView");
  assert.equal(workflowCanvasCommand(key({ key: "n" }), readOnly), null);
  assert.equal(
    workflowCanvasCommand(key({ ctrlKey: true, key: "z" }), readOnly),
    null,
  );
  assert.equal(workflowCanvasCommand(key({ key: "Delete" }), readOnly), null);
});

test("does not steal unrelated modified shortcuts", () => {
  assert.equal(
    workflowCanvasCommand(key({ altKey: true, key: "f" }), editable),
    null,
  );
  assert.equal(
    workflowCanvasCommand(key({ ctrlKey: true, key: "f" }), editable),
    null,
  );
  assert.equal(workflowCanvasCommand(key({ key: "1" }), editable), null);
  assert.equal(
    workflowCanvasCommand(key({ key: "Escape" }), editable),
    "deselect",
  );
});
