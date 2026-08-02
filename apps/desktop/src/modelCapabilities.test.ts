import assert from "node:assert/strict";
import test from "node:test";

import type * as ModelCapabilitiesModule from "./modelCapabilities";
import type { ProviderSettings } from "./types";

const { modelSupportsVision, modelVisionSupportSource } = (await import(
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
    supportsVision: true,
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
});

test("falls back to the legacy connection setting for missing metadata", () => {
  const settings = provider();
  settings.supportsVision = false;

  assert.equal(modelSupportsVision(settings, "unreported-model"), false);
  assert.equal(modelVisionSupportSource(settings, "unreported-model"), "legacy");
});
