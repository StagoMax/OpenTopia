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

/*
 * Mirrors the `[data-theme="dark"]` block in tokens.css. When the user has not
 * edited a field, applyAppearance leaves the corresponding custom property
 * alone so the stylesheet's hand-tuned ramp stays in charge.
 */
export const defaultDarkTheme: ThemeOverrides = {
  accent: "#5AA9F8",
  background: "#181818",
  foreground: "#ECECEC",
  uiFont: DEFAULT_UI_FONT,
  codeFont: DEFAULT_CODE_FONT,
  translucentSidebar: true,
  contrast: 45,
};

/**
 * Dark color triples that shipped as defaults in earlier builds.
 *
 * A stored theme matching one of these was never actually chosen — it is just a
 * captured default. Left alone it would read as a deliberate customization and
 * pin the retired palette forever, so the colors are refreshed to the current
 * default while any other edits (fonts, contrast, sidebar) are preserved.
 */
const retiredDarkColors: ReadonlyArray<
  Pick<ThemeOverrides, "accent" | "background" | "foreground">
> = [
  { accent: "#4C9BEA", background: "#181818", foreground: "#F2F3F5" },
  { accent: "#5AA9F8", background: "#212121", foreground: "#ECECEC" },
  { accent: "#5AA9F8", background: "#141518", foreground: "#ECEEF2" },
];

function refreshRetiredDarkColors(theme: ThemeOverrides): ThemeOverrides {
  const retired = retiredDarkColors.some(
    (old) =>
      old.accent === theme.accent &&
      old.background === theme.background &&
      old.foreground === theme.foreground,
  );
  if (!retired) return theme;
  return {
    ...theme,
    accent: defaultDarkTheme.accent,
    background: defaultDarkTheme.background,
    foreground: defaultDarkTheme.foreground,
  };
}

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

/*
 * v1 predates the "only override what changed" model and is not read. Later
 * palette changes are handled by refreshRetiredDarkColors rather than another
 * key bump, so a genuinely customized theme survives them.
 */
const storageKey = "opentopia.appearance.v2";

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
    dark: refreshRetiredDarkColors(
      normalizeThemeOverrides(raw.dark, defaults.dark),
    ),
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
 * Inline custom properties are only written for fields the user actually
 * changed. An untouched field is *removed* from the inline style so the
 * hand-tuned ramp in tokens.css stays in charge — deriving every role
 * mechanically from two colors produces a flatter, muddier result than the
 * stylesheet does, and it silently shadowed those values before.
 *
 * Colors are validated as hex before they are written, so a hand-edited
 * storage payload cannot inject arbitrary CSS through this path.
 */
export function applyAppearance(
  settings: AppearanceSettings,
  root: HTMLElement = document.documentElement,
): ResolvedTheme {
  const resolved = resolveTheme(settings.mode);
  const theme = resolved === "dark" ? settings.dark : settings.light;
  const base = resolved === "dark" ? defaultDarkTheme : defaultLightTheme;

  root.dataset.theme = resolved;
  root.dataset.diffMarkers = settings.diffMarkers;
  root.dataset.pointerCursor = settings.pointerCursor ? "on" : "off";
  root.dataset.translucentSidebar = theme.translucentSidebar ? "on" : "off";
  if (settings.reduceMotion === "system") delete root.dataset.reduceMotion;
  else root.dataset.reduceMotion = settings.reduceMotion;

  const accent = normalizeHexColor(theme.accent, base.accent);
  const background = normalizeHexColor(theme.background, base.background);
  const foreground = normalizeHexColor(theme.foreground, base.foreground);

  const style = root.style;
  /** Sets an override, or clears it so tokens.css wins again. */
  const put = (name: string, value: string | null) => {
    if (value === null) style.removeProperty(name);
    else style.setProperty(name, value);
  };

  const accentEdited = accent !== base.accent;
  const surfaceEdited =
    background !== base.background || foreground !== base.foreground;
  const bordersEdited = surfaceEdited || theme.contrast !== base.contrast;

  put("--accent", accentEdited ? accent : null);
  put("--focus-ring", accentEdited ? accent : null);
  put(
    "--accent-hover",
    accentEdited ? `color-mix(in srgb, ${accent} 82%, ${foreground})` : null,
  );
  put(
    "--accent-subtle",
    accentEdited || surfaceEdited
      ? `color-mix(in srgb, ${accent} 16%, ${background})`
      : null,
  );

  // Work surfaces sit above the page in dark mode, which is what gives cards
  // their edge; in light mode the page is already the lightest thing there is.
  const elevation = resolved === "dark" ? 5 : 0;
  put("--app-bg", surfaceEdited ? background : null);
  put(
    "--surface",
    surfaceEdited
      ? elevation
        ? `color-mix(in srgb, ${foreground} ${elevation}%, ${background})`
        : background
      : null,
  );
  put(
    "--surface-subtle",
    surfaceEdited
      ? `color-mix(in srgb, ${foreground} ${elevation + 6}%, ${background})`
      : null,
  );
  put(
    "--surface-hover",
    surfaceEdited
      ? `color-mix(in srgb, ${foreground} ${elevation + 9}%, ${background})`
      : null,
  );
  put(
    "--surface-active",
    surfaceEdited
      ? `color-mix(in srgb, ${foreground} ${elevation + 14}%, ${background})`
      : null,
  );

  put("--text", surfaceEdited ? foreground : null);
  put(
    "--text-secondary",
    surfaceEdited
      ? `color-mix(in srgb, ${foreground} 82%, ${background})`
      : null,
  );
  put(
    "--text-muted",
    surfaceEdited
      ? `color-mix(in srgb, ${foreground} 62%, ${background})`
      : null,
  );

  const mix = borderMixPercent(theme.contrast);
  put(
    "--border-subtle",
    bordersEdited
      ? `color-mix(in srgb, ${foreground} ${Math.max(5, mix - 6)}%, ${background})`
      : null,
  );
  put(
    "--border",
    bordersEdited
      ? `color-mix(in srgb, ${foreground} ${mix}%, ${background})`
      : null,
  );
  put(
    "--border-strong",
    bordersEdited
      ? `color-mix(in srgb, ${foreground} ${Math.min(72, mix + 14)}%, ${background})`
      : null,
  );

  put("--font-sans", theme.uiFont === base.uiFont ? null : theme.uiFont);
  put("--font-mono", theme.codeFont === base.codeFont ? null : theme.codeFont);
  put(
    "--font-size-base",
    settings.uiFontSize === defaultAppearanceSettings.uiFontSize
      ? null
      : `${settings.uiFontSize}px`,
  );
  put(
    "--font-size-code",
    settings.codeFontSize === defaultAppearanceSettings.codeFontSize
      ? null
      : `${settings.codeFontSize}px`,
  );

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
