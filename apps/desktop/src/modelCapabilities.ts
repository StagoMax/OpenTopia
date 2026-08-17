import type { ProviderSettings } from "./types";
import modelCapabilityRegistryJson from "../../../crates/opentopia-core/model-capabilities.json" with { type: "json" };

type ModelCapabilityRegistry = {
  schemaVersion: number;
  models: Record<
    string,
    {
      contextWindowTokens: number;
      supportsVision: boolean;
    }
  >;
};

const modelCapabilityRegistry =
  modelCapabilityRegistryJson as ModelCapabilityRegistry;

if (modelCapabilityRegistry.schemaVersion !== 1) {
  throw new Error("Unsupported shared model capability registry schema");
}

export type AutomaticModelVisionSupportSource =
  "detected" | "official" | "unknown";

export type ModelVisionSupportSource =
  "manual" | AutomaticModelVisionSupportSource;

export type ModelVisionSupportResolution = {
  supportsVision: boolean;
  source: ModelVisionSupportSource;
  automaticSource: AutomaticModelVisionSupportSource;
  automaticSupportsVision: boolean | null;
  detectedSupportsVision: boolean | null;
};

/**
 * Candidate model names after removing relay vendor prefixes and modifiers.
 * This mirrors the Rust runtime normalization in `settings.rs` so both sides
 * resolve the same checked-in model profile.
 */
function modelBaseCandidates(modelId: string): string[] {
  const normalized = modelId.trim().toLowerCase();
  const afterSlash = normalized.split("/").at(-1) ?? normalized;
  const afterVariant = afterSlash.split(":", 1)[0];
  const candidates = [afterVariant];
  const dotIndex = afterVariant.indexOf(".");
  if (dotIndex > 0) {
    const prefix = afterVariant.slice(0, dotIndex);
    const rest = afterVariant.slice(dotIndex + 1);
    if (/^[a-z]+$/.test(prefix) && /^[a-z]/.test(rest)) {
      candidates.push(rest);
    }
  }
  return candidates;
}

/** Exact-model fallback shared with the Rust runtime. */
export function knownModelSupportsVision(modelId: string): boolean | undefined {
  for (const candidate of modelBaseCandidates(modelId)) {
    const capabilities = modelCapabilityRegistry.models[candidate];
    if (capabilities) return capabilities.supportsVision;
  }
  return undefined;
}

/**
 * Resolves image-input support without guessing. Explicit per-model settings
 * win over catalog metadata, which wins over the shared checked-in registry;
 * models absent from every source stay unknown and fail closed.
 */
export function resolveModelVisionSupport(
  provider: ProviderSettings,
  modelId: string | null | undefined,
): ModelVisionSupportResolution {
  const id = modelId?.trim() || provider.model.trim();
  const manualSupport = provider.modelSettings?.[id]?.supportsVision;
  const detectedSupport = provider.modelCapabilities?.[id]?.supportsVision;
  const officialSupport = knownModelSupportsVision(id);
  const automaticSupport = detectedSupport ?? officialSupport;
  const automaticSource: AutomaticModelVisionSupportSource =
    detectedSupport !== undefined
      ? "detected"
      : officialSupport !== undefined
        ? "official"
        : "unknown";

  return {
    supportsVision: manualSupport ?? automaticSupport ?? false,
    source: manualSupport !== undefined ? "manual" : automaticSource,
    automaticSource,
    automaticSupportsVision: automaticSupport ?? null,
    detectedSupportsVision: detectedSupport ?? null,
  };
}

/**
 * Resolves image support for one model. Explicit settings win over metadata
 * returned by the provider and the checked-in registry; unknown models are
 * treated as unsupported.
 */
export function modelSupportsVision(
  provider: ProviderSettings,
  modelId: string | null | undefined,
): boolean {
  return resolveModelVisionSupport(provider, modelId).supportsVision;
}

export function modelVisionSupportSource(
  provider: ProviderSettings,
  modelId: string,
): ModelVisionSupportSource {
  return resolveModelVisionSupport(provider, modelId).source;
}
