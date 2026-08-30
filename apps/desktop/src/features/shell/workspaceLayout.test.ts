import assert from "node:assert/strict";
import test from "node:test";

import type * as WorkspaceLayoutModule from "./workspaceLayout";

const { resolveWorkspaceLayout } = (await import(
  "./workspaceLayout" + ".ts"
)) as typeof WorkspaceLayoutModule;

test("Flow inspector uses its compact default width", () => {
  const layout = resolveWorkspaceLayout({}, 1440, "inspector", false);

  assert.equal(layout.right, 360);
  assert.equal(layout.rightMin, 280);
  assert.equal(layout.rightMax, 520);
});

test("Flow inspector width is independent from the code tool stage", () => {
  const preferences = { inspectorRight: 392, toolRight: 760 };

  assert.equal(
    resolveWorkspaceLayout(preferences, 1600, "inspector", false).right,
    392,
  );
  assert.equal(
    resolveWorkspaceLayout(preferences, 1600, "tool", false).right,
    760,
  );
});

test("Flow inspector preference is clamped to the available center space", () => {
  const layout = resolveWorkspaceLayout(
    { inspectorRight: 900 },
    1024,
    "inspector",
    false,
  );

  assert.equal(layout.right, layout.rightMax);
  assert.ok(layout.right >= layout.rightMin);
});
