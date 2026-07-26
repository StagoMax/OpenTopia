/**
 * Appearance preferences for the desktop shell.
 *
 * These are renderer-local: they never round-trip through the Rust server
 * because they describe how this machine draws the app, not how an agent runs.
 * The storage shape mirrors `taskNotifications.ts` so both preference modules
 * validate field by field and survive a partial or corrupt payload.
 */

export type ThemeMode = "system" | "light" | "dark";
export type ResolvedTheme = "light" | "dark";
export type MotionPreference = "system" | "on" | "off";
export type DiffMarkers = "color" | "sign";

/** The per-theme half of the settings, duplicated for light and dark. */
export type ThemeOverrides = {
  accent: string;
  background: string;
  foreground: string;
  uiFont: string;
  codeFont: string;
  translucentSidebar: boolean;
  /** 0-100. Scales how strongly borders separate from their surface. */
  contrast: number;
};

export type AppearanceSettings = {
  mode: ThemeMode;
  light: ThemeOverrides;
  dark: ThemeOverrides;
  pointerCursor: boolean;
  reduceMotion: MotionPreference;
  uiFontSize: number;
  codeFontSize: number;
  diffMarkers: DiffMarkers;
};

export const UI_FONT_SIZE_RANGE = { min: 11, max: 20 } as const;
export const CODE_FONT_SIZE_RANGE = { min: 10, max: 20 } as const;

const DEFAULT_UI_FONT =
  '"Segoe UI Variable Text", "Segoe UI", ui-sans-serif, system-ui, sans-serif';
const DEFAULT_CODE_FONT =
  '"Cascadia Mono", ui-monospace, "SFMono-Regular", Consolas, monospace';

export const defaultLightTheme: ThemeOverrides = {
  accent: "#0B6FD3",
  background: "#FFFFFF",
  foreground: "#25272A",
  uiFont: DEFAULT_UI_FONT,
  codeFont: DEFAULT_CODE_FONT,
  translucentSidebar: true,
  contrast: 45,
};

export const defaultDarkTheme: ThemeOverrides = {
  accent: "#4C9BEA",
  background: "#181818",
  foreground: "#F2F3F5",
  uiFont: DEFAULT_UI_FONT,
  codeFont: DEFAULT_CODE_FONT,
  translucentSidebar: true,
  contrast: 60,
};

export const defaultAppearanceSettings: AppearanceSettings = {
  mode: "system",
  light: defaultLightTheme,
  dark: defaultDarkTheme,
  pointerCursor: false,
  reduceMotion: "system",
  uiFontSize: 14,
  codeFontSize: 12,
  diffMarkers: "color",
};

const storageKey = "opentopia.appearance.v1";

const HEX_COLOR = /^#(?:[0-9a-fA-F]{3}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$/;

/** Guards the CSS injection path: only literal hex colors reach a style value. */
export function isHexColor(value: string): boolean {
  return HEX_COLOR.test(value.trim());
}

export function normalizeHexColor(value: string, fallback: string): string {
  const trimmed = value.trim();
  return isHexColor(trimmed) ? trimmed.toUpperCase() : fallback;
}

function clampNumber(
  value: unknown,
  min: number,
  max: number,
  fallback: number,
) {
  if (typeof value !== "number" || !Number.isFinite(value)) return fallback;
  return Math.min(max, Math.max(min, Math.round(value)));
}

function pickString(value: unknown, fallback: string): string {
  return typeof value === "string" && value.trim().length > 0
    ? value
    : fallback;
}

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

function normalizeThemeOverrides(
  value: unknown,
  fallback: ThemeOverrides,
): ThemeOverrides {
  const raw = (value ?? {}) as Partial<ThemeOverrides>;
  return {
    accent: normalizeHexColor(
      pickString(raw.accent, fallback.accent),
      fallback.accent,
    ),
    background: normalizeHexColor(
      pickString(raw.background, fallback.background),
      fallback.background,
    ),
    foreground: normalizeHexColor(
      pickString(raw.foreground, fallback.foreground),
      fallback.foreground,
    ),
    uiFont: pickString(raw.uiFont, fallback.uiFont),
    codeFont: pickString(raw.codeFont, fallback.codeFont),
    translucentSidebar: pickBoolean(
      raw.translucentSidebar,
      fallback.translucentSidebar,
    ),
    contrast: clampNumber(raw.contrast, 0, 100, fallback.contrast),
  };
}

export function normalizeAppearanceSettings(
  value: unknown,
): AppearanceSettings {
  const raw = (value ?? {}) as Partial<AppearanceSettings>;
  const defaults = defaultAppearanceSettings;
  return {
    mode: pickEnum(
      raw.mode,
      ["system", "light", "dark"] as const,
      defaults.mode,
    ),
    light: normalizeThemeOverrides(raw.light, defaults.light),
    dark: normalizeThemeOverrides(raw.dark, defaults.dark),
    pointerCursor: pickBoolean(raw.pointerCursor, defaults.pointerCursor),
    reduceMotion: pickEnum(
      raw.reduceMotion,
      ["system", "on", "off"] as const,
      defaults.reduceMotion,
    ),
    uiFontSize: clampNumber(
      raw.uiFontSize,
      UI_FONT_SIZE_RANGE.min,
      UI_FONT_SIZE_RANGE.max,
      defaults.uiFontSize,
    ),
    codeFontSize: clampNumber(
      raw.codeFontSize,
      CODE_FONT_SIZE_RANGE.min,
      CODE_FONT_SIZE_RANGE.max,
      defaults.codeFontSize,
    ),
    diffMarkers: pickEnum(
      raw.diffMarkers,
      ["color", "sign"] as const,
      defaults.diffMarkers,
    ),
  };
}

export function readAppearanceSettings(): AppearanceSettings {
  if (typeof window === "undefined") return defaultAppearanceSettings;
  try {
    return normalizeAppearanceSettings(
      JSON.parse(window.localStorage.getItem(storageKey) ?? "{}"),
    );
  } catch {
    return defaultAppearanceSettings;
  }
}

export function writeAppearanceSettings(settings: AppearanceSettings): void {
  try {
    window.localStorage.setItem(storageKey, JSON.stringify(settings));
  } catch {
    // Appearance stays correct for this session if storage is unavailable.
  }
}

export function systemPrefersDark(): boolean {
  if (typeof window === "undefined" || !window.matchMedia) return false;
  return window.matchMedia("(prefers-color-scheme: dark)").matches;
}

export function resolveTheme(mode: ThemeMode): ResolvedTheme {
  if (mode === "system") return systemPrefersDark() ? "dark" : "light";
  return mode;
}

/**
 * Contrast maps to how far the hairline border is mixed toward the text color.
 * The floor stays visible so a 0 setting still renders separators, because the
 * design system separates regions with borders rather than shadows.
 */
function borderMixPercent(contrast: number): number {
  return Math.round(8 + (contrast / 100) * 30);
}

/**
 * Writes the resolved theme onto <html>.
 *
 * Only the roles the user can actually edit are set inline; everything else
 * keeps cascading from tokens.css so the two stay in sync. Colors are validated
 * as hex before they are written, so a hand-edited storage payload cannot
 * inject arbitrary CSS here.
 */
export function applyAppearance(
  settings: AppearanceSettings,
  root: HTMLElement = document.documentElement,
): ResolvedTheme {
  const resolved = resolveTheme(settings.mode);
  const theme = resolved === "dark" ? settings.dark : settings.light;
  const fallback = resolved === "dark" ? defaultDarkTheme : defaultLightTheme;

  root.dataset.theme = resolved;
  root.dataset.diffMarkers = settings.diffMarkers;
  root.dataset.pointerCursor = settings.pointerCursor ? "on" : "off";
  root.dataset.translucentSidebar = theme.translucentSidebar ? "on" : "off";
  if (settings.reduceMotion === "system") delete root.dataset.reduceMotion;
  else root.dataset.reduceMotion = settings.reduceMotion;

  const accent = normalizeHexColor(theme.accent, fallback.accent);
  const background = normalizeHexColor(theme.background, fallback.background);
  const foreground = normalizeHexColor(theme.foreground, fallback.foreground);

  const style = root.style;
  style.setProperty("--accent", accent);
  style.setProperty(
    "--accent-hover",
    `color-mix(in srgb, ${accent} 82%, ${foreground})`,
  );
  style.setProperty(
    "--accent-subtle",
    `color-mix(in srgb, ${accent} 14%, ${background})`,
  );
  style.setProperty("--focus-ring", accent);
  style.setProperty("--app-bg", background);
  style.setProperty("--surface", background);
  style.setProperty(
    "--surface-subtle",
    `color-mix(in srgb, ${foreground} 4%, ${background})`,
  );
  style.setProperty(
    "--surface-hover",
    `color-mix(in srgb, ${foreground} 7%, ${background})`,
  );
  style.setProperty(
    "--surface-active",
    `color-mix(in srgb, ${foreground} 12%, ${background})`,
  );
  style.setProperty("--text", foreground);
  style.setProperty(
    "--text-secondary",
    `color-mix(in srgb, ${foreground} 68%, ${background})`,
  );
  style.setProperty(
    "--text-muted",
    `color-mix(in srgb, ${foreground} 48%, ${background})`,
  );

  const mix = borderMixPercent(theme.contrast);
  style.setProperty(
    "--border-subtle",
    `color-mix(in srgb, ${foreground} ${Math.max(4, mix - 6)}%, ${background})`,
  );
  style.setProperty(
    "--border",
    `color-mix(in srgb, ${foreground} ${mix}%, ${background})`,
  );
  style.setProperty(
    "--border-strong",
    `color-mix(in srgb, ${foreground} ${Math.min(72, mix + 14)}%, ${background})`,
  );

  style.setProperty("--font-sans", theme.uiFont);
  style.setProperty("--font-mono", theme.codeFont);
  style.setProperty("--font-size-base", `${settings.uiFontSize}px`);
  style.setProperty("--font-size-code", `${settings.codeFontSize}px`);

  return resolved;
}

/**
 * Re-applies the theme when the OS flips light/dark, but only while the user is
 * on "system" — an explicit choice must not be overridden by the OS.
 */
export function watchSystemTheme(onChange: () => void): () => void {
  if (typeof window === "undefined" || !window.matchMedia) return () => {};
  const query = window.matchMedia("(prefers-color-scheme: dark)");
  query.addEventListener("change", onChange);
  return () => query.removeEventListener("change", onChange);
}

/** Serializes one theme for the "copy theme" action. */
export function serializeTheme(theme: ThemeOverrides): string {
  return JSON.stringify(theme, null, 2);
}

/** Parses a pasted theme payload, keeping any field it cannot validate. */
export function parseTheme(
  text: string,
  fallback: ThemeOverrides,
): ThemeOverrides | null {
  try {
    const parsed = JSON.parse(text) as unknown;
    if (typeof parsed !== "object" || parsed === null) return null;
    return normalizeThemeOverrides(parsed, fallback);
  } catch {
    return null;
  }
}
