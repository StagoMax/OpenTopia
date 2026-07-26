import assert from "node:assert/strict";
import test from "node:test";

import type * as AppearanceModule from "./appearance";

const appearance: typeof AppearanceModule = await import(
  "./appearance" + ".ts"
);

const {
  applyAppearance,
  defaultAppearanceSettings,
  defaultDarkTheme,
  defaultLightTheme,
  isHexColor,
  normalizeAppearanceSettings,
  normalizeHexColor,
  parseTheme,
  resolveTheme,
  serializeTheme,
} = appearance;

/**
 * Stand-in for <html>. applyAppearance only needs `dataset` and `style`, so a
 * pair of plain maps is enough to assert what it writes without a real DOM.
 */
function fakeRoot() {
  const properties = new Map<string, string>();
  return {
    dataset: {} as Record<string, string | undefined>,
    style: {
      setProperty(name: string, value: string) {
        properties.set(name, value);
      },
    },
    properties,
  };
}

function applyTo(
  settings: AppearanceModule.AppearanceSettings,
  root = fakeRoot(),
) {
  const resolved = applyAppearance(settings, root as unknown as HTMLElement);
  return { root, resolved };
}

test("recognizes 3, 6, and 8 digit hex colors and rejects anything else", () => {
  assert.equal(isHexColor("#abc"), true);
  assert.equal(isHexColor("#A1B2C3"), true);
  assert.equal(isHexColor("#A1B2C3FF"), true);
  assert.equal(isHexColor("  #A1B2C3  "), true);
  assert.equal(isHexColor("#A1B2C"), false);
  assert.equal(isHexColor("red"), false);
  assert.equal(isHexColor("rgb(1,2,3)"), false);
  // The reason the guard exists: these must never reach a CSS value.
  assert.equal(isHexColor("#fff; position: fixed"), false);
  assert.equal(isHexColor("var(--accent)"), false);
});

test("falls back to the supplied color when the value is not hex", () => {
  assert.equal(normalizeHexColor("#a1b2c3", "#000001"), "#A1B2C3");
  assert.equal(normalizeHexColor("nonsense", "#000001"), "#000001");
});

test("normalizes a partial payload to defaults field by field", () => {
  const normalized = normalizeAppearanceSettings({
    mode: "dark",
    uiFontSize: 999,
    light: { accent: "#ff0000" },
    diffMarkers: "bogus",
  });

  assert.equal(normalized.mode, "dark");
  // Clamped into range rather than accepted or dropped.
  assert.equal(normalized.uiFontSize, 20);
  assert.equal(normalized.light.accent, "#FF0000");
  // Untouched sibling fields keep their defaults.
  assert.equal(normalized.light.background, defaultLightTheme.background);
  assert.equal(normalized.diffMarkers, "color");
  assert.deepEqual(normalized.dark, defaultDarkTheme);
});

test("normalizes junk and missing input to the full defaults", () => {
  assert.deepEqual(
    normalizeAppearanceSettings(undefined),
    defaultAppearanceSettings,
  );
  assert.deepEqual(
    normalizeAppearanceSettings(null),
    defaultAppearanceSettings,
  );
  assert.deepEqual(normalizeAppearanceSettings(42), defaultAppearanceSettings);
});

test("an explicit mode resolves to itself and ignores the OS", () => {
  assert.equal(resolveTheme("light"), "light");
  assert.equal(resolveTheme("dark"), "dark");
});

test("applies the light half when the mode is light", () => {
  const { root, resolved } = applyTo({
    ...defaultAppearanceSettings,
    mode: "light",
  });

  assert.equal(resolved, "light");
  assert.equal(root.dataset.theme, "light");
  assert.equal(root.properties.get("--accent"), defaultLightTheme.accent);
  assert.equal(root.properties.get("--text"), defaultLightTheme.foreground);
  assert.equal(root.properties.get("--app-bg"), defaultLightTheme.background);
});

test("applies the dark half, and the two halves stay independent", () => {
  const { root, resolved } = applyTo({
    ...defaultAppearanceSettings,
    mode: "dark",
    dark: { ...defaultDarkTheme, accent: "#123456" },
  });

  assert.equal(resolved, "dark");
  assert.equal(root.dataset.theme, "dark");
  assert.equal(root.properties.get("--accent"), "#123456");
  // The light theme's accent must not leak into the dark render.
  assert.notEqual(root.properties.get("--accent"), defaultLightTheme.accent);
});

test("a non-hex color in storage cannot reach a CSS value", () => {
  const { root } = applyTo({
    ...defaultAppearanceSettings,
    mode: "light",
    // Simulates a hand-edited localStorage payload.
    light: { ...defaultLightTheme, accent: "red; position: fixed" },
  });

  assert.equal(root.properties.get("--accent"), defaultLightTheme.accent);
});

test("font sizes are written as px and fonts pass through verbatim", () => {
  const { root } = applyTo({
    ...defaultAppearanceSettings,
    mode: "light",
    uiFontSize: 16,
    codeFontSize: 13,
    light: { ...defaultLightTheme, uiFont: "Inter", codeFont: "IBM Plex Mono" },
  });

  assert.equal(root.properties.get("--font-size-base"), "16px");
  assert.equal(root.properties.get("--font-size-code"), "13px");
  assert.equal(root.properties.get("--font-sans"), "Inter");
  assert.equal(root.properties.get("--font-mono"), "IBM Plex Mono");
});

test("contrast moves the border away from the surface monotonically", () => {
  function borderFor(contrast: number) {
    const { root } = applyTo({
      ...defaultAppearanceSettings,
      mode: "light",
      light: { ...defaultLightTheme, contrast },
    });
    const value = root.properties.get("--border") ?? "";
    const percent = /(\d+)%/.exec(value)?.[1];
    return Number(percent);
  }

  const low = borderFor(0);
  const mid = borderFor(50);
  const high = borderFor(100);

  assert.ok(low > 0, "a 0 setting still renders a visible separator");
  assert.ok(mid > low);
  assert.ok(high > mid);
});

test("reduce-motion uses a data attribute only when it overrides the OS", () => {
  const systemRoot = applyTo({
    ...defaultAppearanceSettings,
    reduceMotion: "system",
  }).root;
  assert.equal(systemRoot.dataset.reduceMotion, undefined);

  const onRoot = applyTo({
    ...defaultAppearanceSettings,
    reduceMotion: "on",
  }).root;
  assert.equal(onRoot.dataset.reduceMotion, "on");
});

test("switching away from an override clears the stale attribute", () => {
  const root = fakeRoot();
  applyTo({ ...defaultAppearanceSettings, reduceMotion: "off" }, root);
  assert.equal(root.dataset.reduceMotion, "off");

  applyTo({ ...defaultAppearanceSettings, reduceMotion: "system" }, root);
  assert.equal(root.dataset.reduceMotion, undefined);
});

test("the preference flags reach the attributes the stylesheet keys on", () => {
  const { root } = applyTo({
    ...defaultAppearanceSettings,
    mode: "light",
    pointerCursor: true,
    diffMarkers: "sign",
    light: { ...defaultLightTheme, translucentSidebar: false },
  });

  assert.equal(root.dataset.pointerCursor, "on");
  assert.equal(root.dataset.diffMarkers, "sign");
  assert.equal(root.dataset.translucentSidebar, "off");
});

test("a serialized theme round-trips through the parser", () => {
  const theme = { ...defaultDarkTheme, accent: "#0F1E2D", contrast: 33 };
  const parsed = parseTheme(serializeTheme(theme), defaultLightTheme);
  assert.deepEqual(parsed, theme);
});

test("rejects unparseable theme payloads and repairs partial ones", () => {
  assert.equal(parseTheme("not json", defaultLightTheme), null);
  assert.equal(
    parseTheme("[1,2,3]", defaultLightTheme)?.accent,
    defaultLightTheme.accent,
  );

  const partial = parseTheme('{"contrast": 12}', defaultLightTheme);
  assert.equal(partial?.contrast, 12);
  assert.equal(partial?.accent, defaultLightTheme.accent);
});
