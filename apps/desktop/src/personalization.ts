/**
 * Custom instructions for this machine.
 *
 * Stored locally alongside the other renderer preferences. The agent runtime
 * personality lives in `AppSettings.agentRuntime` and is edited on the same
 * page, but it round-trips through the server, so the two are kept apart.
 */

export type PersonalizationSettings = {
  customInstructions: string;
};

export const CUSTOM_INSTRUCTIONS_MAX_LENGTH = 4000;

export const defaultPersonalizationSettings: PersonalizationSettings = {
  customInstructions: "",
};

const storageKey = "opentopia.personalization.v1";

export function normalizePersonalizationSettings(
  value: unknown,
): PersonalizationSettings {
  const raw = (value ?? {}) as Partial<PersonalizationSettings>;
  const instructions =
    typeof raw.customInstructions === "string" ? raw.customInstructions : "";
  return {
    customInstructions: instructions.slice(0, CUSTOM_INSTRUCTIONS_MAX_LENGTH),
  };
}

export function readPersonalizationSettings(): PersonalizationSettings {
  if (typeof window === "undefined") return defaultPersonalizationSettings;
  try {
    return normalizePersonalizationSettings(
      JSON.parse(window.localStorage.getItem(storageKey) ?? "{}"),
    );
  } catch {
    return defaultPersonalizationSettings;
  }
}

export function writePersonalizationSettings(
  settings: PersonalizationSettings,
): void {
  try {
    window.localStorage.setItem(storageKey, JSON.stringify(settings));
  } catch {
    // Instructions stay usable for this session if storage is unavailable.
  }
}
