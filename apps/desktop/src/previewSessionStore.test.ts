import assert from "node:assert/strict";
import test from "node:test";

import type * as PreviewSessionStoreModule from "./previewSessionStore";

const { PreviewSessionStore } = (await import(
  "./previewSessionStore" + ".ts"
)) as typeof PreviewSessionStoreModule;

function session(dirty: boolean) {
  return {
    mode: "source" as const,
    draft: dirty ? "changed" : "saved",
    baseline: "saved",
    revision: "revision-1",
    dirty,
    externalChanged: false,
  };
}

test("stores full preview drafts without notifying aggregate consumers", () => {
  const store = new PreviewSessionStore();
  let notifications = 0;
  store.subscribeToDirtySessions(() => notifications++);

  store.set("tab-1", session(true));
  store.set("tab-1", { ...session(true), draft: "changed again" });

  assert.equal(store.get("tab-1")?.draft, "changed again");
  assert.equal(store.hasDirtySessions(), true);
  assert.equal(notifications, 1);
});

test("deleting a preview session discards its draft and updates dirty state", () => {
  const store = new PreviewSessionStore();
  store.set("tab-1", session(true));

  store.delete("tab-1");

  assert.equal(store.get("tab-1"), undefined);
  assert.equal(store.isDirty("tab-1"), false);
  assert.equal(store.hasDirtySessions(), false);
});
