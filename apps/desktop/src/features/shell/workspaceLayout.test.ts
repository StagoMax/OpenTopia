import assert from "node:assert/strict";
import test from "node:test";

import type * as WorkspaceLayoutModule from "./workspaceLayout";

const { resolveWorkspaceLayout } = (await import(
  "./workspaceLayout" + ".ts"
)) as typeof WorkspaceLayoutModule;

test("agent inspector uses its compact default width", () => {
  const layout = resolveWorkspaceLayout({}, 1440, "agent", false);

  assert.equal(layout.right, 360);
  assert.equal(layout.rightMin, 280);
  assert.equal(layout.rightMax, 520);
});

test("agent inspector width is independent from the code tool stage", () => {
  const preferences = { agentRight: 392, toolRight: 760 };

  assert.equal(
    resolveWorkspaceLayout(preferences, 1600, "agent", false).right,
    392,
  );
  assert.equal(
    resolveWorkspaceLayout(preferences, 1600, "tool", false).right,
    760,
  );
});

test("agent inspector preference is clamped to the available center space", () => {
  const layout = resolveWorkspaceLayout(
    { agentRight: 900 },
    1024,
    "agent",
    false,
  );

  assert.equal(layout.right, layout.rightMax);
  assert.ok(layout.right >= layout.rightMin);
});
