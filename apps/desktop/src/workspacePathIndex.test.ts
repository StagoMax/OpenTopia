import assert from "node:assert/strict";
import test from "node:test";

import type * as WorkspacePathIndexModule from "./workspacePathIndex";

const workspacePathIndex: typeof WorkspacePathIndexModule = await import(
  "./workspacePathIndex" + ".ts"
);

const { WorkspacePathIndex, toWorkspaceRelativePath } = workspacePathIndex;

function indexOf(
  listings: Record<string, string[]>,
  options: { workspaceRoot?: string | null; now?(): number } = {},
) {
  const calls: string[] = [];
  const index = new WorkspacePathIndex({
    workspaceRoot: options.workspaceRoot ?? "J:/Project/OpenTopia",
    now: options.now,
    async listDirectory(directory) {
      calls.push(directory);
      const entries = listings[directory];
      if (!entries) throw new Error(`missing directory: ${directory}`);
      return entries;
    },
  });
  return { calls, index };
}

async function settle(): Promise<void> {
  await new Promise((resolve) => setImmediate(resolve));
}

test("maps absolute and relative paths onto the workspace", () => {
  const root = "J:\\Project\\OpenTopia";
  assert.equal(
    toWorkspaceRelativePath("J:/project/opentopia/docs/plan.md", root),
    "docs/plan.md",
  );
  assert.equal(toWorkspaceRelativePath("./docs/plan.md", root), "docs/plan.md");
  assert.equal(toWorkspaceRelativePath("D:/elsewhere/plan.md", root), null);
  assert.equal(toWorkspaceRelativePath("../outside.md", root), null);
  assert.equal(toWorkspaceRelativePath("/srv/app/main.rs", null), null);
  assert.equal(
    toWorkspaceRelativePath("/srv/app/main.rs", "/srv/app"),
    "main.rs",
  );
});

test("reports a path as known only after the directory is read", async () => {
  const { calls, index } = indexOf({ docs: ["plan.md", "notes.md"] });
  assert.equal(index.status("docs/plan.md"), "unknown");

  index.watch("docs/plan.md", () => {});
  await settle();
  assert.deepEqual(calls, ["docs"]);
  assert.equal(index.status("docs/plan.md"), "known");
  assert.equal(index.status("docs/missing.md"), "missing");
});

test("notifies watchers and shares one listing per directory", async () => {
  const { calls, index } = indexOf({ docs: ["plan.md"] });
  let notifications = 0;
  index.watch("docs/plan.md", () => {
    notifications += 1;
  });
  index.watch("docs/other.md", () => {
    notifications += 1;
  });
  await settle();

  assert.deepEqual(calls, ["docs"]);
  assert.equal(notifications, 2);
});

test("treats paths outside the workspace as missing without a lookup", async () => {
  const { calls, index } = indexOf({ docs: ["plan.md"] });
  index.watch("D:/elsewhere/plan.md", () => {});
  await settle();
  assert.deepEqual(calls, []);
  assert.equal(index.status("D:/elsewhere/plan.md"), "missing");
});

test("re-reads a directory only after a missing result goes stale", async () => {
  let clock = 0;
  const listings: Record<string, string[]> = { docs: [] };
  const { calls, index } = indexOf(listings, { now: () => clock });

  index.watch("docs/plan.md", () => {});
  await settle();
  assert.equal(index.status("docs/plan.md"), "missing");

  clock = 5_000;
  index.watch("docs/plan.md", () => {});
  await settle();
  assert.deepEqual(calls, ["docs"]);

  clock = 20_000;
  listings.docs = ["plan.md"];
  index.watch("docs/plan.md", () => {});
  await settle();
  assert.deepEqual(calls, ["docs", "docs"]);
  assert.equal(index.status("docs/plan.md"), "known");
});

test("keeps a known path cached without further listings", async () => {
  const { calls, index } = indexOf({ "": ["README.md"] });
  index.watch("README.md", () => {});
  await settle();
  index.watch("README.md", () => {});
  await settle();
  assert.deepEqual(calls, [""]);
  assert.equal(index.status("README.md"), "known");
});

test("keeps unreadable directories from linking anything", async () => {
  const { index } = indexOf({});
  index.watch("nope/plan.md", () => {});
  await settle();
  assert.equal(index.status("nope/plan.md"), "missing");
});
