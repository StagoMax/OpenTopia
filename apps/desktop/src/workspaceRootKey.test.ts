import assert from "node:assert/strict";
import test from "node:test";

import type * as WorkspaceRootKeyModule from "./workspaceRootKey";

const { workspaceRootKey } = (await import(
  "./workspaceRootKey" + ".ts"
)) as typeof WorkspaceRootKeyModule;

test("normalizes Windows workspace aliases into a stable key", () => {
  assert.equal(workspaceRootKey("C:\\Work\\Repo\\"), "c:/work/repo");
  assert.equal(workspaceRootKey("c:/work//repo"), "c:/work/repo");
  assert.equal(
    workspaceRootKey("\\\\?\\UNC\\Server\\Share\\Repo"),
    "//server/share/repo",
  );
});
