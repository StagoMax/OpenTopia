import assert from "node:assert/strict";
import test from "node:test";

import {
  agentKnowledgeBindingSummary,
  agentKnowledgeProvider,
  agentToolsWithKnowledgeAccess,
} from "./agentKnowledgeBinding.ts";

test("treats legacy namespace-only Agent bindings as SAG", () => {
  const binding = { namespaces: ["opentopia.audit.credit-review.v1"] };

  assert.equal(agentKnowledgeProvider(binding), "sag");
  assert.equal(
    agentKnowledgeBindingSummary(binding),
    "SAG · opentopia.audit.credit-review.v1",
  );
});

test("derives library_search from the Agent knowledge selection", () => {
  assert.deepEqual(agentToolsWithKnowledgeAccess(["shell"], "graph-rag"), [
    "shell",
    "library_search",
  ]);
  assert.deepEqual(agentToolsWithKnowledgeAccess(["shell"], ""), ["shell"]);
});

test("represents Graph RAG as an Agent provider without a fixed database", () => {
  const binding = { provider: "graph-rag" as const, namespaces: [] };

  assert.equal(agentKnowledgeProvider(binding), "graph-rag");
  assert.equal(agentKnowledgeBindingSummary(binding), "Graph RAG");
});
