import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

const topBarStylesPath = new URL(
  "../../styles/app-legacy-layout.css",
  import.meta.url,
);

test("the custom title-bar menus are not enclosed by a draggable Electron region", async () => {
  const styles = await readFile(topBarStylesPath, "utf8");
  const topbar = styles.match(/\.topbar\s*\{(?<rules>[^}]*)\}/)?.groups?.rules;
  const menu = styles.match(/\.window-menu\s*\{(?<rules>[^}]*)\}/)?.groups?.rules;

  assert.ok(topbar, "the topbar styles should be present");
  assert.ok(menu, "the window menu styles should be present");
  assert.doesNotMatch(topbar, /-webkit-app-region\s*:\s*drag/);
  assert.match(styles, /\.topbar::after\s*\{[\s\S]*?-webkit-app-region\s*:\s*drag/);
  assert.match(menu, /position\s*:\s*relative/);
  assert.match(menu, /z-index\s*:\s*var\(--z-sticky\)/);
  assert.match(menu, /-webkit-app-region\s*:\s*no-drag/);
});
