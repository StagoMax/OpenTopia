import assert from "node:assert/strict";
import test from "node:test";

import type * as EditorPreferencesModule from "./editorPreferences";

const editorPreferences: typeof EditorPreferencesModule = await import(
  "./editorPreferences" + ".ts"
);

const {
  defaultEditorPreferences,
  normalizeEditorPreferences,
  shouldSubmitOnKey,
} = editorPreferences;

type Key = Parameters<typeof shouldSubmitOnKey>[0];

function key(overrides: Partial<Key> = {}): Key {
  return {
    key: "Enter",
    shiftKey: false,
    ctrlKey: false,
    metaKey: false,
    ...overrides,
  };
}

test("normalizes a partial payload without dropping valid fields", () => {
  const normalized = normalizeEditorPreferences({
    sendShortcut: "mod-enter",
    followUpBehavior: "nope",
    showBottomPanel: "yes",
  });

  assert.equal(normalized.sendShortcut, "mod-enter");
  // Invalid enum and wrong-typed boolean both fall back.
  assert.equal(normalized.followUpBehavior, "queue");
  assert.equal(normalized.showBottomPanel, false);
});

test("normalizes junk and missing input to the full defaults", () => {
  assert.deepEqual(
    normalizeEditorPreferences(undefined),
    defaultEditorPreferences,
  );
  assert.deepEqual(
    normalizeEditorPreferences("nonsense"),
    defaultEditorPreferences,
  );
});

test("in Enter mode a bare Enter submits and a modifier does not", () => {
  assert.equal(shouldSubmitOnKey(key(), "enter"), true);
  assert.equal(shouldSubmitOnKey(key({ ctrlKey: true }), "enter"), false);
  assert.equal(shouldSubmitOnKey(key({ metaKey: true }), "enter"), false);
});

test("in modifier mode only Ctrl/Cmd+Enter submits", () => {
  assert.equal(shouldSubmitOnKey(key(), "mod-enter"), false);
  assert.equal(shouldSubmitOnKey(key({ ctrlKey: true }), "mod-enter"), true);
  assert.equal(shouldSubmitOnKey(key({ metaKey: true }), "mod-enter"), true);
});

test("Shift+Enter always inserts a newline in both modes", () => {
  assert.equal(shouldSubmitOnKey(key({ shiftKey: true }), "enter"), false);
  assert.equal(shouldSubmitOnKey(key({ shiftKey: true }), "mod-enter"), false);
  assert.equal(
    shouldSubmitOnKey(key({ shiftKey: true, ctrlKey: true }), "mod-enter"),
    false,
  );
});

test("no other key submits", () => {
  assert.equal(shouldSubmitOnKey(key({ key: "a" }), "enter"), false);
  assert.equal(shouldSubmitOnKey(key({ key: "Tab" }), "enter"), false);
  assert.equal(
    shouldSubmitOnKey(key({ key: "a", ctrlKey: true }), "mod-enter"),
    false,
  );
});
