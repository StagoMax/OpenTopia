import assert from "node:assert/strict";
import test from "node:test";

import type * as ProviderSettingsModelModule from "./providerSettingsModel";

const {
  completedModelDiscoveryState,
  contextWindowInputConstraints,
  contextWindowPresets,
  createProviderSettings,
  hasCachedProviderModelCatalog,
  providerChangeInvalidatesModelDiscovery,
} = (await import(
  "./providerSettingsModel" + ".ts"
)) as typeof ProviderSettingsModelModule;

test("keeps an imported catalog when capability negotiation is unavailable", () => {
  assert.deepEqual(
    completedModelDiscoveryState({
      models: ["model-a", "model-b"],
      defaultModelReady: false,
      capabilityWarning: "HTTP 429",
    }),
    {
      status: "warning",
      modelCount: 2,
      message: "HTTP 429",
    },
  );
});

test("reports full success only after the default model is ready", () => {
  assert.deepEqual(
    completedModelDiscoveryState({
      models: ["model-a"],
      defaultModelReady: true,
      capabilityWarning: undefined,
    }),
    { status: "success", modelCount: 1 },
  );
});

test("accepts every context-window preset in the manual input", () => {
  const { min, step } = contextWindowInputConstraints;

  for (const preset of contextWindowPresets) {
    assert.ok(Number.isInteger(preset.tokens), preset.label);
    assert.ok(preset.tokens >= min, preset.label);
    assert.equal((preset.tokens - min) % step, 0, preset.label);
  }
});

test("uses the binary token value for the 1M context preset", () => {
  assert.equal(
    contextWindowPresets.find(
      (preset) => preset.label === "1M（1,048,576 tokens）",
    )?.tokens,
    1_048_576,
  );
});

test("labels every context-window preset with its exact token count", () => {
  for (const preset of contextWindowPresets) {
    assert.ok(
      preset.label.includes(preset.tokens.toLocaleString("en-US")),
      preset.label,
    );
  }
});

test("reuses a persisted model catalog when reopening settings", () => {
  const provider = createProviderSettings("relay", {
    modelsSyncedAt: "2026-08-20T08:00:00.000Z",
    syncedModels: [],
  });

  assert.equal(hasCachedProviderModelCatalog(provider), true);
  assert.equal(
    hasCachedProviderModelCatalog(
      createProviderSettings("not-yet-synced", { modelsSyncedAt: null }),
    ),
    false,
  );
  assert.equal(
    hasCachedProviderModelCatalog(
      createProviderSettings("legacy-provider", { modelsSyncedAt: undefined }),
    ),
    false,
  );
});

test("invalidates the cached catalog only for discovery-relevant changes", () => {
  for (const field of [
    "baseUrl",
    "model",
    "kind",
    "transport",
    "auth",
    "allowedAdapters",
    "preferredAdapter",
  ] as const) {
    assert.equal(providerChangeInvalidatesModelDiscovery(field), true, field);
  }

  for (const field of ["name", "enabledFamilies", "temperature"] as const) {
    assert.equal(providerChangeInvalidatesModelDiscovery(field), false, field);
  }
});
