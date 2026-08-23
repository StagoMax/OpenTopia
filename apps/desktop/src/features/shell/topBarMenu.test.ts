import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

const topBarStylesPath = new URL(
  "../../styles/app-legacy-layout.css",
  import.meta.url,
);
const topBarPath = new URL("./TopBar.tsx", import.meta.url);

test("the custom title-bar menus use a separate trailing Electron drag region", async () => {
  const [styles, topBar] = await Promise.all([
    readFile(topBarStylesPath, "utf8"),
    readFile(topBarPath, "utf8"),
  ]);
  const topbar = styles.match(/\.topbar\s*\{(?<rules>[^}]*)\}/)?.groups?.rules;
  const menu = styles.match(/\.window-menu\s*\{(?<rules>[^}]*)\}/)?.groups?.rules;
  const dragRegion = styles.match(
    /\.topbar-drag-region\s*\{(?<rules>[^}]*)\}/,
  )?.groups?.rules;

  assert.ok(topbar, "the topbar styles should be present");
  assert.ok(menu, "the window menu styles should be present");
  assert.ok(
    dragRegion,
    "the dedicated draggable region styles should be present",
  );
  assert.doesNotMatch(topbar, /-webkit-app-region\s*:\s*drag/);
  assert.match(topbar, /z-index\s*:\s*var\(--z-sticky\)/);
  assert.match(topbar, /-webkit-app-region\s*:\s*no-drag/);
  assert.doesNotMatch(styles, /\.topbar::after\s*\{/);
  assert.match(topBar, /className="topbar-drag-region"/);
  assert.match(dragRegion, /flex\s*:\s*1\s+1\s+auto/);
  assert.match(dragRegion, /-webkit-app-region\s*:\s*drag/);
  assert.match(menu, /position\s*:\s*relative/);
  assert.match(menu, /z-index\s*:\s*var\(--z-sticky\)/);
  assert.match(menu, /-webkit-app-region\s*:\s*no-drag/);
});
