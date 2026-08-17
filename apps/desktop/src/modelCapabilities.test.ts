import assert from "node:assert/strict";
import test from "node:test";

import type * as ModelCapabilitiesModule from "./modelCapabilities";
import type { ProviderSettings } from "./types";

const {
  knownModelSupportsVision,
  modelSupportsVision,
  modelVisionSupportSource,
  resolveModelVisionSupport,
} = (await import(
  "./modelCapabilities" + ".ts"
)) as typeof ModelCapabilitiesModule;

function provider(): ProviderSettings {
  return {
    id: "relay",
    name: "Relay",
    kind: "openai_compatible",
    baseUrl: "https://models.example/v1",
    model: "text-only",
    enabledFamilies: [],
    syncedModels: ["text-only", "vision"],
    modelContextWindows: {},
    modelCapabilities: {
      "text-only": { supportsVision: false },
      vision: { supportsVision: true },
    },
    modelSettings: {},
    modelsSyncedAt: null,
    contextWindowTokens: null,
    storeResponses: false,
    parallelToolCalls: false,
    apiKeySource: "OPENTOPIA_API_KEY",
    apiKeyConfigured: true,
  };
}

test("uses detected vision support for the selected model", () => {
  const settings = provider();
  assert.equal(modelSupportsVision(settings, "text-only"), false);
  assert.equal(modelVisionSupportSource(settings, "vision"), "detected");
});

test("keeps an explicit per-model override above discovered metadata", () => {
  const settings = provider();
  settings.modelSettings = { "text-only": { supportsVision: true } };

  assert.equal(modelSupportsVision(settings, "text-only"), true);
  assert.equal(modelVisionSupportSource(settings, "text-only"), "manual");
  assert.deepEqual(resolveModelVisionSupport(settings, "text-only"), {
    supportsVision: true,
    source: "manual",
    automaticSource: "detected",
    automaticSupportsVision: false,
    detectedSupportsVision: false,
  });
});

test("uses the shared official registry when catalog metadata is missing", () => {
  const settings = provider();

  assert.equal(modelSupportsVision(settings, "moonshotai/kimi-k2.5"), true);
  assert.equal(
    modelVisionSupportSource(settings, "moonshotai/kimi-k2.5"),
    "official",
  );
  assert.deepEqual(
    resolveModelVisionSupport(settings, "moonshotai/kimi-k2.5"),
    {
      supportsVision: true,
      source: "official",
      automaticSource: "official",
      automaticSupportsVision: true,
      detectedSupportsVision: null,
    },
  );
});

test("keeps explicit catalog metadata above the shared registry", () => {
  const settings = provider();
  settings.modelCapabilities = {
    ...settings.modelCapabilities,
    "kimi-k2.5": { supportsVision: false },
  };

  assert.deepEqual(resolveModelVisionSupport(settings, "kimi-k2.5"), {
    supportsVision: false,
    source: "detected",
    automaticSource: "detected",
    automaticSupportsVision: false,
    detectedSupportsVision: false,
  });
});

test("covers major vendors without family-wide guessing", () => {
  for (const model of [
    "openai/gpt-5.6-sol:batch",
    "anthropic/claude-sonnet-4.6",
    "google/gemini-3.5-flash",
    "k3-256k",
    "kimi-for-coding",
    "qwen/qwen3.7-plus",
    "z-ai/glm-5v-turbo",
    "x-ai/grok-4.5",
    "mistralai/mistral-small-3.2-24b-instruct",
    "meta-llama/llama-4-scout",
    "minimax/minimax-m3",
  ]) {
    assert.equal(knownModelSupportsVision(model), true, model);
  }

  for (const model of [
    "kimi-k2",
    "moonshot-v1-128k",
    "deepseek-v4-flash",
    "qwen3-coder-plus",
    "glm-5",
    "minimax-m2.5",
  ]) {
    assert.equal(knownModelSupportsVision(model), false, model);
  }
  assert.equal(knownModelSupportsVision("custom/unknown-vlm"), undefined);
});

test("keeps missing image metadata unknown and fails closed", () => {
  const settings = provider();

  assert.equal(modelSupportsVision(settings, "unreported-model"), false);
  assert.equal(
    modelVisionSupportSource(settings, "unreported-model"),
    "unknown",
  );
  assert.deepEqual(resolveModelVisionSupport(settings, "unreported-model"), {
    supportsVision: false,
    source: "unknown",
    automaticSource: "unknown",
    automaticSupportsVision: null,
    detectedSupportsVision: null,
  });
});
