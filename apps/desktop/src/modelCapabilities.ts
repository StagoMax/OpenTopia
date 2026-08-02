import type { ProviderSettings } from "./types";

/**
 * Resolves image support for one model. Explicit settings win over metadata
 * returned by the provider, while legacy connections retain their previous
 * connection-wide fallback until their catalog exposes capabilities.
 */
export function modelSupportsVision(
  provider: ProviderSettings,
  modelId: string | null | undefined,
): boolean {
  const id = modelId?.trim() || provider.model.trim();
  return (
    provider.modelSettings?.[id]?.supportsVision ??
    provider.modelCapabilities?.[id]?.supportsVision ??
    provider.supportsVision
  );
}

export function modelVisionSupportSource(
  provider: ProviderSettings,
  modelId: string,
): "manual" | "detected" | "legacy" {
  const id = modelId.trim();
  if (provider.modelSettings?.[id]?.supportsVision !== undefined) {
    return "manual";
  }
  if (provider.modelCapabilities?.[id]?.supportsVision !== undefined) {
    return "detected";
  }
  return "legacy";
}
