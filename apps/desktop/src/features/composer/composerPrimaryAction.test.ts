import assert from "node:assert/strict";
import test from "node:test";

import { resolveComposerPrimaryAction } from "./composerPrimaryAction.ts";

test("uses the single composer button to append while a task is running", () => {
  assert.equal(
    resolveComposerPrimaryAction({
      hasSendableContent: true,
      isSending: false,
      isRunning: true,
    }),
    "submit",
  );
});

test("uses the single composer button to stop only when the running draft is empty", () => {
  assert.equal(
    resolveComposerPrimaryAction({
      hasSendableContent: false,
      isSending: false,
      isRunning: true,
    }),
    "cancel",
  );
});

test("keeps the sending state when submitting an idle draft", () => {
  assert.equal(
    resolveComposerPrimaryAction({
      hasSendableContent: true,
      isSending: true,
      isRunning: false,
    }),
    "sending",
  );
});
