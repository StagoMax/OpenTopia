import assert from "node:assert/strict";
import test from "node:test";

import type * as ToolTabStateModule from "./toolTabState";

const toolTabState: typeof ToolTabStateModule = await import(
  "./toolTabState" + ".ts"
);

const { closeToolTabState } = toolTabState;

test("collapses the tool panel after its last tab closes", () => {
  assert.deepEqual(
    closeToolTabState([{ id: "terminal" }], "terminal", "terminal"),
    {
      tabs: [],
      activeTabId: null,
      shouldCollapse: true,
    },
  );
});

test("keeps the tool panel open and activates a neighbor when tabs remain", () => {
  assert.deepEqual(
    closeToolTabState(
      [{ id: "files" }, { id: "diff" }, { id: "terminal" }],
      "diff",
      "diff",
    ),
    {
      tabs: [{ id: "files" }, { id: "terminal" }],
      activeTabId: "terminal",
      shouldCollapse: false,
    },
  );
});

test("keeps the active tab when a background tab closes", () => {
  assert.deepEqual(
    closeToolTabState(
      [{ id: "files" }, { id: "terminal" }],
      "terminal",
      "files",
    ),
    {
      tabs: [{ id: "terminal" }],
      activeTabId: "terminal",
      shouldCollapse: false,
    },
  );
});
