/**
 * Renderer-level preferences for the review panel.
 *
 * The reader picks a diff view, wrapping, and the display switches once and
 * expects them to survive a restart, so they live on this machine rather than
 * in thread state. Validated field by field like the other preference modules
 * so a hand-edited payload degrades to the default instead of throwing.
 */

export type DiffReviewView = "split" | "unified";

export type DiffReviewPreferences = {
  view: DiffReviewView;
  /** Wrap long lines instead of scrolling the diff horizontally. */
  wrapLines: boolean;
  /** Load the whole working-tree file so untouched regions can be expanded. */
  loadFullFile: boolean;
  /** Render Markdown and other rich formats instead of their source diff. */
  richPreview: boolean;
  /** Highlight the words that changed inside a replaced line. */
  wordDiff: boolean;
  /** Treat whitespace-only edits as untouched. */
  hideWhitespace: boolean;
  /** Show the changed-file tree beside the diff. */
  showFilePanel: boolean;
};

export const defaultDiffReviewPreferences: DiffReviewPreferences = {
  view: "unified",
  wrapLines: false,
  loadFullFile: false,
  richPreview: false,
  wordDiff: false,
  hideWhitespace: false,
  showFilePanel: false,
};

const storageKey = "opentopia.diffReviewPreferences.v1";

function pickBoolean(value: unknown, fallback: boolean): boolean {
  return typeof value === "boolean" ? value : fallback;
}

export function normalizeDiffReviewPreferences(
  value: unknown,
): DiffReviewPreferences {
  const raw = (value ?? {}) as Partial<DiffReviewPreferences>;
  const defaults = defaultDiffReviewPreferences;
  return {
    view:
      raw.view === "unified" || raw.view === "split" ? raw.view : defaults.view,
    wrapLines: pickBoolean(raw.wrapLines, defaults.wrapLines),
    loadFullFile: pickBoolean(raw.loadFullFile, defaults.loadFullFile),
    richPreview: pickBoolean(raw.richPreview, defaults.richPreview),
    wordDiff: pickBoolean(raw.wordDiff, defaults.wordDiff),
    hideWhitespace: pickBoolean(raw.hideWhitespace, defaults.hideWhitespace),
    showFilePanel: pickBoolean(raw.showFilePanel, defaults.showFilePanel),
  };
}

export function readDiffReviewPreferences(): DiffReviewPreferences {
  if (typeof window === "undefined") return defaultDiffReviewPreferences;
  try {
    return normalizeDiffReviewPreferences(
      JSON.parse(window.localStorage.getItem(storageKey) ?? "{}"),
    );
  } catch {
    return defaultDiffReviewPreferences;
  }
}

export function writeDiffReviewPreferences(
  preferences: DiffReviewPreferences,
): void {
  try {
    window.localStorage.setItem(storageKey, JSON.stringify(preferences));
  } catch {
    // Preferences stay usable for this session if storage is unavailable.
  }
}
