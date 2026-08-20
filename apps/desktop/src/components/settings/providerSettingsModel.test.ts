import assert from "node:assert/strict";
import test from "node:test";

import type * as ProviderSettingsModelModule from "./providerSettingsModel";

const {
  createProviderSettings,
  hasCachedProviderModelCatalog,
  providerChangeInvalidatesModelDiscovery,
} = (await import(
  "./providerSettingsModel" + ".ts"
)) as typeof ProviderSettingsModelModule;

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
