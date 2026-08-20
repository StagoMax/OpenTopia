import assert from "node:assert/strict";
import test from "node:test";

import type * as ComposerModes from "./composerModes";

const { normalizedPermissionMode, permissionModeOptions } = (await import(
  "./composerModes" + ".ts"
)) as typeof ComposerModes;

test("keeps the full-access appearance independent of the stored permission value", () => {
  const option = permissionModeOptions.find(
    ({ value }) => value === "unrestricted",
  );

  assert.equal(option?.appearance, "full-access");
  assert.equal(normalizedPermissionMode("full_access"), "unrestricted");
});
