import assert from "node:assert/strict";
import test from "node:test";
import { nextFlowEditorStage } from "./flowEditorProgress.ts";

test("the Flow editor has one execution test between validation and activation", () => {
  assert.equal(
    nextFlowEditorStage({
      draftExists: true,
      successfulTestRun: false,
      validated: true,
    }),
    "test",
  );
  assert.equal(
    nextFlowEditorStage({
      draftExists: true,
      successfulTestRun: true,
      validated: true,
    }),
    "activate",
  );
});

test("saving and validation remain prerequisites for Test Run", () => {
  assert.equal(
    nextFlowEditorStage({
      draftExists: false,
      successfulTestRun: false,
      validated: false,
    }),
    "save",
  );
  assert.equal(
    nextFlowEditorStage({
      draftExists: true,
      successfulTestRun: false,
      validated: false,
    }),
    "validate",
  );
});
