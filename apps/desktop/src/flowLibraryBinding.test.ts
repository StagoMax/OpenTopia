import assert from "node:assert/strict";
import test from "node:test";

import type * as FlowLibraryBindingModule from "./flowLibraryBinding";

const {
  FLOW_LIBRARY_PROVIDER_OPTIONS,
  flowLibraryProviderLabel,
  resolveFlowLibraryProvider,
  updateFlowLibraryBindings,
} = (await import(
  "./flowLibraryBinding" + ".ts"
)) as typeof FlowLibraryBindingModule;

test("uses the draft Lib before a Flow thread exists", () => {
  assert.equal(resolveFlowLibraryProvider(null, {}, "sag"), "sag");
});

test("uses the thread binding after the draft becomes a thread", () => {
  assert.equal(
    resolveFlowLibraryProvider("thread-1", { "thread-1": "graph-rag" }, "sag"),
    "graph-rag",
  );
});

test("adds, changes, and removes a thread Lib binding", () => {
  const added = updateFlowLibraryBindings({}, "thread-1", "sag");
  assert.deepEqual(added, { "thread-1": "sag" });

  const changed = updateFlowLibraryBindings(added, "thread-1", "graph-rag");
  assert.deepEqual(changed, { "thread-1": "graph-rag" });

  const removed = updateFlowLibraryBindings(changed, "thread-1", null);
  assert.deepEqual(removed, {});
});

test("offers provider-only Flow deployment choices", () => {
  assert.deepEqual(
    FLOW_LIBRARY_PROVIDER_OPTIONS.map((option) => option.value),
    ["", "graph-rag", "sag"],
  );
  assert.equal(flowLibraryProviderLabel("graph-rag"), "Graph RAG");
  assert.equal(flowLibraryProviderLabel(null), "未启用");
});
