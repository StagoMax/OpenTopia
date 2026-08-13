import assert from "node:assert/strict";
import test from "node:test";
import type { SagSource } from "./types";
import {
  coverageByNeed,
  filterSagSources,
  parseSagMetadata,
  sagErrorMessage,
} from "./sagLibrary.ts";

const source: SagSource = {
  assetId: "ast_1",
  sourceKey: "policies/publishing",
  namespace: "enterprise_knowledge",
  origin: "upload",
  versionId: "ver_1",
  versionNumber: 2,
  sourceId: "src_1",
  title: "内容发布规范",
  originalFilename: "publishing.docx",
  contentHash: "hash",
  storedPath: "assets/publishing.docx",
  metadata: {},
  evidenceUnits: 8,
  events: 12,
  createdAt: "2026-08-13T00:00:00Z",
};

test("filters SAG sources across human and stable identifiers", () => {
  assert.equal(filterSagSources([source], "发布").length, 1);
  assert.equal(filterSagSources([source], "policies/").length, 1);
  assert.equal(filterSagSources([source], "missing").length, 0);
});

test("indexes coverage by the retrieval need id", () => {
  const indexed = coverageByNeed([
    {
      needId: "latest_policy",
      required: true,
      status: "covered",
      selectedEventIds: ["evt_1"],
      reason: "selected-evidence",
    },
  ]);
  assert.equal(indexed.get("latest_policy")?.status, "covered");
});

test("accepts only object metadata", () => {
  assert.deepEqual(parseSagMetadata('{"department":"sales"}'), {
    department: "sales",
  });
  assert.throws(() => parseSagMetadata("[]"), /JSON 对象/);
});

test("extracts the OpenTopia API error envelope", () => {
  assert.equal(
    sagErrorMessage(new Error('{"error":"SAG 服务未启动"}')),
    "SAG 服务未启动",
  );
});
