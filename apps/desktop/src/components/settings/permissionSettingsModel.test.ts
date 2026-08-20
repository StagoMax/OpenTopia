import assert from "node:assert/strict";
import test from "node:test";

import type * as PermissionSettingsModel from "./permissionSettingsModel";
import type { AppSettings } from "../../types";

const {
  approvalStrategyMode,
  permissionAccessMode,
  selectPermissionAccessMode,
  selectPermissionMode,
  systemSandboxIsActive,
} = (await import(
  "./permissionSettingsModel" + ".ts"
)) as typeof PermissionSettingsModel;

const sandbox: AppSettings["sandbox"] = {
  sandboxMode: "workspace-write",
  enforcement: "enforce",
  network: "deny",
  writableRoots: [],
  readPaths: [],
};

test("keeps the legacy full_access value as guarded host access", () => {
  const selected = selectPermissionAccessMode(
    "guarded-full-access",
    "unrestricted",
    sandbox,
  );
  assert.equal(selected.sandbox.sandboxMode, "danger-full-access");
  assert.equal(selected.sandbox.enforcement, "disabled");
  assert.equal(selected.sandbox.network, "allow");
  assert.equal(
    permissionAccessMode(selected.permissionMode, selected.sandbox),
    "guarded-full-access",
  );
  assert.equal(approvalStrategyMode(selected.permissionMode), "unrestricted");
});

test("maps complete system access to the no-approval unrestricted mode", () => {
  const selected = selectPermissionMode("unrestricted", sandbox);
  assert.equal(selected.permissionMode, "unrestricted");
  assert.equal(
    permissionAccessMode(selected.permissionMode, selected.sandbox),
    "unrestricted",
  );
  assert.equal(approvalStrategyMode(selected.permissionMode), "unrestricted");
  assert.equal(systemSandboxIsActive(selected.sandbox), false);
});

test("shows system sandbox status only while isolation is active", () => {
  assert.equal(systemSandboxIsActive(sandbox), true);
  assert.equal(
    systemSandboxIsActive({ ...sandbox, enforcement: "disabled" }),
    false,
  );
});

test("leaving host access restores a controlled approval and sandbox preset", () => {
  const selected = selectPermissionAccessMode(
    "read-only",
    "unrestricted",
    selectPermissionMode("unrestricted", sandbox).sandbox,
  );
  assert.equal(selected.permissionMode, "auto");
  assert.equal(selected.sandbox.sandboxMode, "read-only");
  assert.equal(selected.sandbox.enforcement, "enforce");
});
