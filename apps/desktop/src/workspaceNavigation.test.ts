import assert from "node:assert/strict";
import test from "node:test";

import type * as WorkspaceNavigationModule from "./workspaceNavigation";

const { resolveActiveFlowPrimaryView, resolveSidebarDestination } =
  (await import(
    "./workspaceNavigation" + ".ts"
  )) as typeof WorkspaceNavigationModule;

test("maps fixed Flow navigation to one primary destination", () => {
  for (const [flowPrimaryView, destination] of [
    ["overview", "flow-overview"],
    ["agents", "flow-agents"],
    ["workflow-templates", "flow-workflow-templates"],
    ["automation", "flow-automation"],
    ["runs", "flow-runs"],
    ["trust", "flow-trust"],
  ] as const) {
    assert.equal(
      resolveSidebarDestination({
        experienceMode: "flow",
        flowPrimaryView,
        toolStageOpen: false,
        activeToolKind: null,
      }),
      destination,
    );
  }
  assert.equal(
    resolveSidebarDestination({
      experienceMode: "flow",
      flowPrimaryView: "deployments",
      toolStageOpen: false,
      activeToolKind: null,
    }),
    "flow-deployments",
  );
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
  const sidebarDestination = resolveSidebarDestination({
    experienceMode: "flow",
    flowPrimaryView: "trust",
    toolStageOpen: true,
    activeToolKind: "extensions",
  });

  assert.equal(sidebarDestination, "plugins");
  assert.equal(
    resolveActiveFlowPrimaryView({
      flowPrimaryView: "trust",
      sidebarDestination,
    }),
    null,
  );
});

test("marks the active Flow page when it owns the sidebar destination", () => {
  assert.equal(
    resolveActiveFlowPrimaryView({
      flowPrimaryView: "trust",
      sidebarDestination: "flow-trust",
    }),
    "trust",
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
