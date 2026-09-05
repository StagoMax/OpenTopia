import assert from "node:assert/strict";
import test from "node:test";
import type { SagSource } from "../../types";

import {
  loadSagNamespaceOptions,
  parseSagNamespaceSelection,
  toggleSagNamespaceSelection,
} from "./sagNamespaceOptions.ts";

test("loads and counts SAG namespaces across source pages", async () => {
  const requestedOffsets: number[] = [];
  const client = {
    async listLibrarySources(
      provider: "sag" | "graph-rag",
      options: { offset?: number; limit?: number },
    ) {
      assert.equal(provider, "sag");
      requestedOffsets.push(options.offset ?? 0);
      const offset = options.offset ?? 0;
      const namespaces = offset === 0 ? ["work", "credit"] : ["work"];
      return {
        items: namespaces.map((namespace, index) =>
          source(namespace, `${offset}-${index}`),
        ),
        total: 3,
        authorizedTotal: 3,
        indexTotal: 3,
        offset,
        limit: options.limit ?? 200,
        hasMore: offset === 0,
      };
    },
  };

  assert.deepEqual(await loadSagNamespaceOptions(client), [
    { namespace: "credit", sourceCount: 1 },
    { namespace: "work", sourceCount: 2 },
  ]);
  assert.deepEqual(requestedOffsets, [0, 2]);
});

function source(namespace: string, id: string): SagSource {
  return {
    assetId: `asset-${id}`,
    sourceKey: `source-${id}`,
    namespace,
    origin: "upload",
    versionId: `version-${id}`,
    versionNumber: 1,
    sourceId: id,
    title: id,
    originalFilename: `${id}.md`,
    contentHash: `hash-${id}`,
    storedPath: `${id}.md`,
    metadata: {},
    evidenceUnits: 1,
    events: 1,
    createdAt: "2026-09-05T00:00:00Z",
  };
}

test("parses, adds, and removes namespace selections without duplicates", () => {
  assert.deepEqual(parseSagNamespaceSelection("work, credit\nwork"), [
    "work",
    "credit",
  ]);
  assert.equal(
    toggleSagNamespaceSelection("manual, work", "credit", true),
    "manual, work, credit",
  );
  assert.equal(
    toggleSagNamespaceSelection("manual, work", "work", false),
    "manual",
  );
});
