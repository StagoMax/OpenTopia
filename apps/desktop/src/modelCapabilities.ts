import type { ProviderSettings, ThreadModelSelection } from "./types";
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

export type ModelContextWindowSource =
  | "model_override"
  | "connection_override"
  | "detected"
  | "official"
  | "inferred"
  | "fallback";

export type ModelContextWindowResolution = {
  contextWindowTokens: number;
  source: ModelContextWindowSource;
  inferredFromModelId?: string;
};

type KnownModelContextWindowResolution = {
  contextWindowTokens: number;
  source: "official" | "inferred";
  referenceModelId: string;
};

type ModelGeneration = {
  prefix: string;
  version: number[];
  suffix: string;
};

// Keep this aligned with `DEFAULT_UNKNOWN_MODEL_CONTEXT_WINDOW_TOKENS` in the
// Rust provider settings. The shared registry supplies known model values;
// this remains the deliberately conservative result for unknown models.
export const DEFAULT_UNKNOWN_MODEL_CONTEXT_WINDOW_TOKENS = 128_000;

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

function isAsciiDigit(value: string, index: number): boolean {
  const code = value.charCodeAt(index);
  return code >= 48 && code <= 57;
}

function isAsciiLetter(value: string, index: number): boolean {
  const code = value.charCodeAt(index);
  return (code >= 65 && code <= 90) || (code >= 97 && code <= 122);
}

/**
 * Splits a model id into the named series before its generation, the numeric
 * generation itself, and the model variant after it. A number followed by a
 * unit such as `120b` is a parameter size, not a generation, so it is not a
 * safe basis for inference.
 */
function modelGeneration(modelId: string): ModelGeneration | undefined {
  for (let start = 0; start < modelId.length; start += 1) {
    if (!isAsciiDigit(modelId, start)) continue;

    const version: number[] = [];
    let cursor = start;
    while (cursor < modelId.length) {
      const partStart = cursor;
      while (cursor < modelId.length && isAsciiDigit(modelId, cursor)) {
        cursor += 1;
      }
      version.push(Number(modelId.slice(partStart, cursor)));

      if (modelId[cursor] !== "." || !isAsciiDigit(modelId, cursor + 1)) {
        break;
      }
      cursor += 1;
    }

    if (isAsciiLetter(modelId, cursor)) continue;
    return {
      prefix: modelId.slice(0, start),
      version,
      suffix: modelId.slice(cursor),
    };
  }
  return undefined;
}

function compareModelGenerations(left: number[], right: number[]): number {
  const length = Math.max(left.length, right.length);
  for (let index = 0; index < length; index += 1) {
    const difference = (left[index] ?? 0) - (right[index] ?? 0);
    if (difference !== 0) return difference;
  }
  return 0;
}

function inferredContextWindow(
  generation: ModelGeneration,
): KnownModelContextWindowResolution | undefined {
  let closest:
    | (KnownModelContextWindowResolution & {
        version: number[];
      })
    | undefined;

  for (const [modelId, capabilities] of Object.entries(
    modelCapabilityRegistry.models,
  )) {
    const candidate = modelGeneration(modelId);
    if (
      !candidate ||
      candidate.prefix !== generation.prefix ||
      candidate.suffix !== generation.suffix ||
      compareModelGenerations(candidate.version, generation.version) >= 0 ||
      (closest &&
        compareModelGenerations(candidate.version, closest.version) <= 0)
    ) {
      continue;
    }
    closest = {
      contextWindowTokens: capabilities.contextWindowTokens,
      source: "inferred",
      referenceModelId: modelId,
      version: candidate.version,
    };
  }

  if (!closest) return undefined;
  const { version: _version, ...resolution } = closest;
  return resolution;
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
 * Resolves the shared registry's exact model record, then a same-series,
 * previous-generation record when the model id is newer than this build. The
 * variant must be identical, so a new Flash model never borrows Pro, Image,
 * or Coder settings.
 */
export function resolveKnownModelContextWindow(
  modelId: string,
): KnownModelContextWindowResolution | undefined {
  const candidates = modelBaseCandidates(modelId);
  for (const candidate of candidates) {
    const capabilities = modelCapabilityRegistry.models[candidate];
    if (capabilities) {
      return {
        contextWindowTokens: capabilities.contextWindowTokens,
        source: "official",
        referenceModelId: candidate,
      };
    }
  }

  for (const candidate of candidates) {
    const generation = modelGeneration(candidate);
    if (!generation) continue;
    const inferred = inferredContextWindow(generation);
    if (inferred) return inferred;
  }
  return undefined;
}

/** Context-window value from the shared registry, including safe inference. */
export function knownModelContextWindowTokens(
  modelId: string,
): number | undefined {
  return resolveKnownModelContextWindow(modelId)?.contextWindowTokens;
}

/**
 * Resolves the effective context window using the same precedence as the
 * runtime, while preserving the source so settings can explain the automatic
 * result instead of merely saying that one will be used.
 */
export function resolveModelContextWindow(
  provider: ProviderSettings,
  modelId: string | null | undefined,
): ModelContextWindowResolution {
  const id = modelId?.trim() || provider.model.trim();
  const modelOverride = provider.modelSettings?.[id]?.contextWindowTokens;
  if (modelOverride != null) {
    return { contextWindowTokens: modelOverride, source: "model_override" };
  }

  if (provider.contextWindowTokens != null) {
    return {
      contextWindowTokens: provider.contextWindowTokens,
      source: "connection_override",
    };
  }

  const detectedContextWindow = provider.modelContextWindows?.[id];
  if (detectedContextWindow != null) {
    return {
      contextWindowTokens: detectedContextWindow,
      source: "detected",
    };
  }

  const knownContextWindow = resolveKnownModelContextWindow(id);
  if (knownContextWindow) {
    return {
      contextWindowTokens: knownContextWindow.contextWindowTokens,
      source: knownContextWindow.source,
      ...(knownContextWindow.source === "inferred"
        ? { inferredFromModelId: knownContextWindow.referenceModelId }
        : {}),
    };
  }

  return {
    contextWindowTokens: DEFAULT_UNKNOWN_MODEL_CONTEXT_WINDOW_TOKENS,
    source: "fallback",
  };
}

/**
 * Resolves the context window for the model a thread actually uses. A stale
 * pinned connection falls back to the active connection, matching the model
 * selector instead of applying the stale model id to an unrelated provider.
 */
export function resolveThreadModelContextWindow(
  providers: ProviderSettings[],
  activeProviderId: string | null | undefined,
  selection: ThreadModelSelection | null | undefined,
): ModelContextWindowResolution | null {
  const provider =
    providers.find((item) => item.id === selection?.connectionId) ??
    providers.find((item) => item.id === activeProviderId) ??
    providers[0];
  if (!provider) return null;

  const modelId =
    selection?.connectionId === provider.id
      ? selection.modelId
      : provider.model;
  return resolveModelContextWindow(provider, modelId);
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
