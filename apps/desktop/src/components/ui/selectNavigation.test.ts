import assert from "node:assert/strict";
import test from "node:test";

import {
  firstEnabledOptionIndex,
  lastEnabledOptionIndex,
  moveEnabledOptionIndex,
  selectedOrFirstEnabledOptionIndex,
} from "./selectNavigation.ts";

const options = [
  { value: "webhook" },
  { value: "schedule", disabled: true },
  { value: "event" },
];

test("finds the selected enabled option or a usable fallback", () => {
  assert.equal(selectedOrFirstEnabledOptionIndex(options, "event"), 2);
  assert.equal(selectedOrFirstEnabledOptionIndex(options, "schedule"), 0);
  assert.equal(selectedOrFirstEnabledOptionIndex(options, "missing"), 0);
});

test("moves through enabled options and wraps around disabled ones", () => {
  assert.equal(firstEnabledOptionIndex(options), 0);
  assert.equal(lastEnabledOptionIndex(options), 2);
  assert.equal(moveEnabledOptionIndex(options, 0, 1), 2);
  assert.equal(moveEnabledOptionIndex(options, 0, -1), 2);
  assert.equal(
    moveEnabledOptionIndex([{ value: "only", disabled: true }], 0, 1),
    -1,
  );
});
