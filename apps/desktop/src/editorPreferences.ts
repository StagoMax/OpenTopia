/**
 * Renderer-level editor and shell preferences surfaced on the General page.
 *
 * These describe how this machine presents the composer and workspace chrome.
 * Like the other preference modules they are validated field by field so a
 * partial or hand-edited payload degrades to the default instead of throwing.
 */

/** Whether Enter submits, or inserts a newline and requires a modifier to send. */
export type SendShortcut = "enter" | "mod-enter";

/** What a message typed while a turn is running does. */
export type FollowUpBehavior = "queue" | "steer";

export type EditorPreferences = {
  showContextWindowUsage: boolean;
  sendShortcut: SendShortcut;
  followUpBehavior: FollowUpBehavior;
  showBottomPanel: boolean;
  allowProjectlessTasks: boolean;
};

export const defaultEditorPreferences: EditorPreferences = {
  showContextWindowUsage: false,
  sendShortcut: "enter",
  followUpBehavior: "queue",
  showBottomPanel: false,
  allowProjectlessTasks: false,
};

const storageKey = "opentopia.editorPreferences.v1";

function pickBoolean(value: unknown, fallback: boolean): boolean {
  return typeof value === "boolean" ? value : fallback;
}

function pickEnum<T extends string>(
  value: unknown,
  allowed: readonly T[],
  fallback: T,
): T {
  return typeof value === "string" &&
    (allowed as readonly string[]).includes(value)
    ? (value as T)
    : fallback;
}

export function normalizeEditorPreferences(value: unknown): EditorPreferences {
  const raw = (value ?? {}) as Partial<EditorPreferences>;
  const defaults = defaultEditorPreferences;
  return {
    showContextWindowUsage: pickBoolean(
      raw.showContextWindowUsage,
      defaults.showContextWindowUsage,
    ),
    sendShortcut: pickEnum(
      raw.sendShortcut,
      ["enter", "mod-enter"] as const,
      defaults.sendShortcut,
    ),
    followUpBehavior: pickEnum(
      raw.followUpBehavior,
      ["queue", "steer"] as const,
      defaults.followUpBehavior,
    ),
    showBottomPanel: pickBoolean(raw.showBottomPanel, defaults.showBottomPanel),
    allowProjectlessTasks: pickBoolean(
      raw.allowProjectlessTasks,
      defaults.allowProjectlessTasks,
    ),
  };
}

export function readEditorPreferences(): EditorPreferences {
  if (typeof window === "undefined") return defaultEditorPreferences;
  try {
    return normalizeEditorPreferences(
      JSON.parse(window.localStorage.getItem(storageKey) ?? "{}"),
    );
  } catch {
    return defaultEditorPreferences;
  }
}

export function writeEditorPreferences(preferences: EditorPreferences): void {
  try {
    window.localStorage.setItem(storageKey, JSON.stringify(preferences));
  } catch {
    // Preferences stay usable for this session if storage is unavailable.
  }
}

/**
 * Whether a keydown in the composer should submit, given the send shortcut.
 * Shift+Enter always inserts a newline, matching every chat composer.
 */
export function shouldSubmitOnKey(
  event: Pick<KeyboardEvent, "key" | "shiftKey" | "ctrlKey" | "metaKey">,
  shortcut: SendShortcut,
): boolean {
  if (event.key !== "Enter" || event.shiftKey) return false;
  const modifier = event.ctrlKey || event.metaKey;
  return shortcut === "enter" ? !modifier : modifier;
}
