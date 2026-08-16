import assert from "node:assert/strict";
import test from "node:test";

import type * as WorkspaceNavigationModule from "./workspaceNavigation";

const { resolveSidebarDestination } = (await import(
  "./workspaceNavigation" + ".ts"
)) as typeof WorkspaceNavigationModule;

test("marks the Flow Library as the current sidebar destination", () => {
  assert.equal(
    resolveSidebarDestination({
      experienceMode: "flow",
      flowPrimaryView: "library",
      toolStageOpen: false,
      activeToolKind: null,
    }),
    "flow-library",
  );
});

test("lets the full-workspace Plugins page own the current state", () => {
  assert.equal(
    resolveSidebarDestination({
      experienceMode: "flow",
      flowPrimaryView: "library",
      toolStageOpen: true,
      activeToolKind: "extensions",
    }),
    "plugins",
  );
});

test("keeps contextual tool stages attached to their primary destination", () => {
  assert.equal(
    resolveSidebarDestination({
      experienceMode: "flow",
      flowPrimaryView: "library",
      toolStageOpen: true,
      activeToolKind: "browser",
    }),
    "flow-library",
  );
  assert.equal(
    resolveSidebarDestination({
      experienceMode: "code",
      flowPrimaryView: "library",
      toolStageOpen: true,
      activeToolKind: "terminal",
    }),
    "conversation",
  );
});
