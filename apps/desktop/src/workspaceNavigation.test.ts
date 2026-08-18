import assert from "node:assert/strict";
import test from "node:test";

import type * as WorkspaceNavigationModule from "./workspaceNavigation";

const { resolveSidebarDestination } = (await import(
  "./workspaceNavigation" + ".ts"
)) as typeof WorkspaceNavigationModule;

test("maps fixed Flow navigation to one primary destination", () => {
  assert.equal(
    resolveSidebarDestination({
      experienceMode: "flow",
      flowPrimaryView: "inbox",
      toolStageOpen: false,
      activeToolKind: null,
    }),
    "flow-inbox",
  );
  assert.equal(
    resolveSidebarDestination({
      experienceMode: "flow",
      flowPrimaryView: "connections",
      toolStageOpen: false,
      activeToolKind: null,
    }),
    "flow-connections",
  );
  assert.equal(
    resolveSidebarDestination({
      experienceMode: "flow",
      flowPrimaryView: "knowledge",
      toolStageOpen: false,
      activeToolKind: null,
    }),
    "flow-knowledge",
  );
});

test("lets the full-workspace Plugins page own the current state", () => {
  assert.equal(
    resolveSidebarDestination({
      experienceMode: "flow",
      flowPrimaryView: "connections",
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
      flowPrimaryView: "knowledge",
      toolStageOpen: true,
      activeToolKind: "browser",
    }),
    "flow-knowledge",
  );
  assert.equal(
    resolveSidebarDestination({
      experienceMode: "code",
      flowPrimaryView: "knowledge",
      toolStageOpen: true,
      activeToolKind: "terminal",
    }),
    "conversation",
  );
});
