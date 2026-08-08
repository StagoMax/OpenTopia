import assert from "node:assert/strict";
import test from "node:test";
import {
  composerExternalValueSyncAction,
  composerInputCommitPending,
} from "./composerContent.ts";

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
