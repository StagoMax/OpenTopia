import assert from "node:assert/strict";
import test from "node:test";
import {
  composerExternalValueSyncAction,
  composerInputCommitPending,
} from "./composerContent.ts";
import { composerEnterCommand } from "./composerInput.ts";

type EnterKey = Parameters<typeof composerEnterCommand>[0];

function enterKey(overrides: Partial<EnterKey> = {}): EnterKey {
  return {
    altKey: false,
    ctrlKey: false,
    key: "Enter",
    metaKey: false,
    shiftKey: false,
    ...overrides,
  };
}

test("resolves default Enter as submit and Shift+Enter as list-aware newline", () => {
  assert.equal(composerEnterCommand(enterKey(), "enter"), "submit");
  assert.equal(
    composerEnterCommand(enterKey({ shiftKey: true }), "enter"),
    "insert-list-line-break",
  );
});

test("does not reinterpret Alt+Enter or non-Enter keys", () => {
  assert.equal(composerEnterCommand(enterKey({ altKey: true }), "enter"), null);
  assert.equal(composerEnterCommand(enterKey({ key: "a" }), "enter"), null);
  assert.equal(
    composerEnterCommand(
      enterKey({ ctrlKey: true, shiftKey: true }),
      "enter",
    ),
    null,
  );
});

test("keeps modifier-send compatibility without changing Shift+Enter", () => {
  assert.equal(
    composerEnterCommand(enterKey(), "mod-enter"),
    "insert-line-break",
  );
  assert.equal(
    composerEnterCommand(enterKey({ ctrlKey: true }), "mod-enter"),
    "submit",
  );
  assert.equal(
    composerEnterCommand(enterKey({ shiftKey: true }), "mod-enter"),
    "insert-list-line-break",
  );
});

test("keeps IME preedit and final input events inside one composition transaction", () => {
  assert.equal(
    composerInputCommitPending({
      isComposing: true,
      compositionSnapshotPending: true,
      nativeIsComposing: true,
    }),
    true,
  );
  assert.equal(
    composerInputCommitPending({
      isComposing: false,
      compositionSnapshotPending: true,
      nativeIsComposing: false,
    }),
    true,
  );
  assert.equal(
    composerInputCommitPending({
      isComposing: false,
      compositionSnapshotPending: false,
      nativeIsComposing: false,
    }),
    false,
  );
});

test("does not feed a locally published value back into the editor", () => {
  assert.equal(
    composerExternalValueSyncAction({
      value: "a",
      lastLocallyPublishedValue: "a",
      compositionPending: false,
    }),
    "ignore",
  );
});

test("keeps the browser's live edit when a queued draft publish rerenders", () => {
  assert.equal(
    composerExternalValueSyncAction({
      value: "before delete",
      lastLocallyPublishedValue: "before delet",
      compositionPending: false,
      pendingLocalPublish: true,
      lastExternalValue: "before delete",
    }),
    "ignore",
  );
});

test("defers a real external value until IME composition has settled", () => {
  assert.equal(
    composerExternalValueSyncAction({
      value: "external reset",
      lastLocallyPublishedValue: "local draft",
      compositionPending: true,
    }),
    "defer",
  );
  assert.equal(
    composerExternalValueSyncAction({
      value: "external reset",
      lastLocallyPublishedValue: "local draft",
      compositionPending: false,
    }),
    "apply",
  );
});
