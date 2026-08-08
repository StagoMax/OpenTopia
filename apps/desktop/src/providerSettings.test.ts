import assert from "node:assert/strict";
import test from "node:test";

import type * as ProviderSettingsModule from "./providerSettings";

const {
  OFFICIAL_OPENAI_MODEL_PRESETS,
  findOfficialModelPreset,
  normalizeProviderNames,
  normalizeProviderReasoningEffort,
  normalizeReasoningEffortForModel,
  providerDisplayName,
  resolveModelReasoningCapability,
} = (await import(
  "./providerSettings" + ".ts"
)) as typeof ProviderSettingsModule;

test("uses a custom provider name without changing its stable ID", () => {
  assert.equal(
    providerDisplayName({ id: "custom-provider-3", name: "Kimi K3" }),
    "Kimi K3",
  );
});

test("normalizes legacy providers that do not have a display name", () => {
  const [provider] = normalizeProviderNames([
    {
      id: "legacy-provider",
      name: "",
      kind: "openai_compatible",
      baseUrl: "https://example.test/v1",
      model: "legacy-model",
      enabledFamilies: [],
      syncedModels: [],
      modelsSyncedAt: null,
      temperature: null,
      maxOutputTokens: null,
      contextWindowTokens: null,
      reasoningEffort: null,
      storeResponses: false,
      parallelToolCalls: false,
      promptCacheKey: null,
      promptCachePolicy: null,
      responsesCompactionThresholdTokens: null,
      rolloutBudget: null,
      supportsVision: true,
      apiKeySource: "OPENTOPIA_API_KEY",
      apiKeyConfigured: false,
      healthStatus: null,
    },
  ]);

  assert.equal(provider.name, "legacy-provider");
  assert.equal(provider.id, "legacy-provider");
});

test("exposes current OpenAI presets with official source links", () => {
  assert.deepEqual(
    OFFICIAL_OPENAI_MODEL_PRESETS.filter(
      (preset) => preset.group === "recommended",
    ).map((preset) => preset.model),
    ["gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna", "gpt-5.6"],
  );
  assert.ok(
    OFFICIAL_OPENAI_MODEL_PRESETS.every((preset) =>
      preset.sourceUrl.startsWith("https://developers.openai.com/"),
    ),
  );
  assert.equal(
    findOfficialModelPreset(" GPT-5.6-TERRA ")?.label,
    "GPT-5.6 Terra",
  );
  assert.equal(findOfficialModelPreset("gpt-5.4-pro")?.label, "GPT-5.4 Pro");
  assert.equal(
    findOfficialModelPreset("gpt-5.3-codex")?.label,
    "GPT-5.3-Codex",
  );
});

test("resolves model-specific reasoning effort capabilities", () => {
  assert.deepEqual(
    resolveModelReasoningCapability("openai_responses", "gpt-5.6-sol")
      .supportedEfforts,
    ["none", "low", "medium", "high", "xhigh", "max"],
  );
  assert.equal(
    resolveModelReasoningCapability("openai_responses", "gpt-5.6-terra")
      .defaultEffort,
    "medium",
  );
  assert.deepEqual(
    resolveModelReasoningCapability("openai_responses", "gpt-5.4-mini")
      .supportedEfforts,
    ["none", "low", "medium", "high", "xhigh"],
  );
  assert.deepEqual(
    resolveModelReasoningCapability("openai_responses", "gpt-5.3-codex")
      .supportedEfforts,
    ["low", "medium", "high", "xhigh"],
  );
  assert.deepEqual(
    resolveModelReasoningCapability("openai_responses", "gpt-5.5")
      .supportedEfforts,
    ["none", "low", "medium", "high", "xhigh"],
  );
  assert.deepEqual(
    resolveModelReasoningCapability("openai_responses", "gpt-5.4-pro")
      .supportedEfforts,
    ["medium", "high", "xhigh"],
  );
});

test("drops invalid known-model effort values but preserves custom compatibility", () => {
  assert.equal(
    normalizeReasoningEffortForModel("openai_responses", "gpt-5.4-mini", "max"),
    null,
  );
  assert.equal(
    normalizeReasoningEffortForModel(
      "openai_responses",
      "gpt-5.3-codex",
      "none",
    ),
    null,
  );
  assert.equal(
    normalizeReasoningEffortForModel(
      "openai_compatible",
      "private-reasoning-model",
      "max",
    ),
    "max",
  );
});

test("keeps Claude effort when its model supports Anthropic's effort parameter", () => {
  const provider = normalizeProviderReasoningEffort({
    id: "anthropic",
    name: "Anthropic",
    kind: "anthropic",
    baseUrl: "https://api.anthropic.com",
    model: "claude-sonnet-4-6",
    enabledFamilies: [],
    syncedModels: [],
    modelsSyncedAt: null,
    temperature: 0.2,
    maxOutputTokens: null,
    contextWindowTokens: null,
    reasoningEffort: "high",
    storeResponses: false,
    parallelToolCalls: false,
    promptCacheKey: null,
    promptCachePolicy: null,
    responsesCompactionThresholdTokens: null,
    rolloutBudget: null,
    supportsVision: true,
    apiKeySource: "ANTHROPIC_API_KEY",
    apiKeyConfigured: false,
    healthStatus: null,
  });

  assert.equal(provider.reasoningEffort, "high");
  assert.deepEqual(
    resolveModelReasoningCapability("anthropic", "claude-opus-4-7")
      .supportedEfforts,
    ["low", "medium", "high", "xhigh", "max"],
  );
  assert.deepEqual(
    resolveModelReasoningCapability("openai_compatible", "k3-256k")
      .supportedEfforts,
    ["none", "low", "medium", "high", "xhigh", "max"],
  );
  assert.equal(
    resolveModelReasoningCapability("openai_compatible", "glm-5.2")
      .defaultEffort,
    "max",
  );
  assert.equal(
    resolveModelReasoningCapability("openai_responses", "gpt-4.1-mini").status,
    "unsupported",
  );
});

test("recognizes DeepSeek V4 dual-mode thinking controls", () => {
  const flash = resolveModelReasoningCapability(
    "openai_compatible",
    "deepseek-v4-flash",
  );
  assert.equal(flash.status, "supported");
  assert.equal(flash.thinkingToggle, true);
  assert.equal(flash.defaultEffort, "high");
  assert.deepEqual(flash.supportedEfforts, ["none", "high", "max"]);

  const prefixed = resolveModelReasoningCapability(
    "openai_compatible",
    "deepseek/deepseek-v4-pro-20260801",
  );
  assert.equal(prefixed.thinkingToggle, true);
  assert.deepEqual(prefixed.supportedEfforts, ["none", "high", "max"]);

  const reasoner = resolveModelReasoningCapability(
    "openai_compatible",
    "deepseek-reasoner",
  );
  assert.deepEqual(reasoner.supportedEfforts, ["high", "max"]);
});
