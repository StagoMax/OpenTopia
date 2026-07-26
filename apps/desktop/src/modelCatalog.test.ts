import assert from "node:assert/strict";
import test from "node:test";

import type * as ModelCatalogModule from "./modelCatalog";

const {
  availableFamiliesForModels,
  buildConnectionModelGroups,
  classifyModelFamily,
  compareModelRecency,
  formatModelDisplayName,
  isPreviewModel,
  reconcileReasoningEffort,
  resolveDefaultModelId,
  resolveReasoningOptions,
} = (await import("./modelCatalog" + ".ts")) as typeof ModelCatalogModule;

test("classifies relay model ids into families", () => {
  assert.equal(classifyModelFamily("kimi-k2.5-turbo"), "kimi");
  assert.equal(classifyModelFamily("moonshot-v1-128k"), "kimi");
  assert.equal(classifyModelFamily("moonshotai/kimi-k2"), "kimi");
  assert.equal(classifyModelFamily("deepseek-reasoner"), "deepseek");
  assert.equal(classifyModelFamily("anthropic.claude-sonnet-4-5"), "claude");
  assert.equal(classifyModelFamily("gpt-5.6-sol"), "gpt");
  assert.equal(classifyModelFamily("o3-mini"), "gpt");
  assert.equal(classifyModelFamily("qwen-max-latest"), "qwen");
  assert.equal(classifyModelFamily("glm-4.6"), "glm");
  assert.equal(classifyModelFamily("gemini-2.5-pro"), "gemini");
});

test("falls back to the other family instead of hiding unknown models", () => {
  assert.equal(classifyModelFamily("my-finetune-v3"), "other");
  assert.equal(classifyModelFamily("internal-router"), "other");
});

test("detects preview and dated snapshot markers", () => {
  assert.ok(isPreviewModel("gemini-2.0-flash-exp"));
  assert.ok(isPreviewModel("claude-opus-4-preview"));
  assert.ok(!isPreviewModel("kimi-k2.5"));
});

test("orders models newest version first", () => {
  const ordered = ["kimi-k2.5", "kimi-k3", "kimi-k2.7"]
    .slice()
    .sort(compareModelRecency);
  assert.deepEqual(ordered, ["kimi-k3", "kimi-k2.7", "kimi-k2.5"]);
});

test("orders dated snapshots of one version newest first", () => {
  const ordered = ["claude-sonnet-4-20240229", "claude-sonnet-4-20250514"]
    .slice()
    .sort(compareModelRecency);
  assert.deepEqual(ordered, [
    "claude-sonnet-4-20250514",
    "claude-sonnet-4-20240229",
  ]);
});

test("marks the newest stable model of each family as latest", () => {
  const groups = buildConnectionModelGroups(
    ["kimi-k2.5", "kimi-k3", "kimi-k3-preview", "deepseek-reasoner"],
    [],
  );
  const kimi = groups.find((group) => group.family.id === "kimi");
  assert.ok(kimi);
  assert.equal(kimi.models.find((model) => model.latest)?.id, "kimi-k3");
});

test("an empty enabled-family list shows every family", () => {
  const groups = buildConnectionModelGroups(["kimi-k3", "gpt-5.6"], []);
  assert.deepEqual(
    groups.map((group) => group.family.id),
    ["gpt", "kimi"],
  );
});

test("enabling a family hides the others", () => {
  const groups = buildConnectionModelGroups(
    ["kimi-k3", "gpt-5.6", "deepseek-reasoner"],
    ["kimi"],
  );
  assert.deepEqual(
    groups.map((group) => group.family.id),
    ["kimi"],
  );
});

test("lists only families the connection actually serves", () => {
  assert.deepEqual(
    availableFamiliesForModels(["kimi-k3", "kimi-k2.5"]).map(
      (family) => family.id,
    ),
    ["kimi"],
  );
});

test("defaults a new thread to the newest stable enabled model", () => {
  assert.equal(
    resolveDefaultModelId(
      ["kimi-k2.5", "kimi-k3", "kimi-k4-preview"],
      ["kimi"],
      "fallback-model",
    ),
    "kimi-k3",
  );
});

test("falls back to the configured model when nothing is enabled", () => {
  assert.equal(
    resolveDefaultModelId(["kimi-k3"], ["claude"], "fallback-model"),
    "fallback-model",
  );
});

test("prettifies ids while preserving version shape", () => {
  assert.equal(formatModelDisplayName("kimi-k2.5-turbo"), "Kimi K2.5 Turbo");
  assert.equal(
    formatModelDisplayName("deepseek-reasoner"),
    "Deepseek Reasoner",
  );
  assert.equal(
    formatModelDisplayName("anthropic/claude-sonnet-4-20250514"),
    "Claude Sonnet 4",
  );
});

test("official OpenAI reasoning rules stay authoritative", () => {
  const capability = resolveReasoningOptions("openai_responses", "gpt-5.6-sol");
  assert.equal(capability.official, true);
  assert.equal(capability.defaultEffort, "medium");
});

test("family rules cover models the official table does not know", () => {
  const capability = resolveReasoningOptions("openai_compatible", "kimi-k3");
  assert.equal(capability.status, "supported");
  assert.deepEqual(capability.supportedEfforts, [
    "none",
    "low",
    "medium",
    "high",
  ]);
});

test("non-reasoning members of a reasoning family offer no efforts", () => {
  assert.deepEqual(
    resolveReasoningOptions("openai_compatible", "deepseek-chat")
      .supportedEfforts,
    [],
  );
  assert.deepEqual(
    resolveReasoningOptions("openai_compatible", "moonshot-v1-128k")
      .supportedEfforts,
    [],
  );
});

test("switching to a model that keeps the effort preserves the user choice", () => {
  assert.equal(
    reconcileReasoningEffort("openai_compatible", "kimi-k3", "high"),
    "high",
  );
});

test("switching to a model without the effort falls back to its default", () => {
  assert.equal(
    reconcileReasoningEffort("openai_responses", "gpt-5.6-sol", "minimal"),
    "medium",
  );
});

test("switching to a non-reasoning model clears the effort", () => {
  assert.equal(
    reconcileReasoningEffort("openai_compatible", "deepseek-chat", "high"),
    null,
  );
});
