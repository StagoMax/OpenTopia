import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const styles = readFileSync(
  new URL("./styles/agent-template-panel.css", import.meta.url),
  "utf8",
);
const panelSource = readFileSync(
  new URL("./components/ui/Panel.tsx", import.meta.url),
  "utf8",
);

test("Flow Agent inspector header keeps actions inset on one row", () => {
  assert.match(
    styles,
    /\.agent-template-panel--rail > \.ot-panel > \.ot-panel__header \{[^}]*min-block-size: calc\(var\(--control-height-lg\) \+ var\(--space-4\)\);[^}]*padding: var\(--space-3\) var\(--space-4\);[^}]*\}/s,
  );
  assert.match(
    styles,
    /\.agent-template-panel--rail \.agent-template-panel__header-actions \{[^}]*flex: 0 0 auto;[^}]*flex-wrap: nowrap;[^}]*gap: var\(--space-2\);[^}]*\}/s,
  );
});

test("Flow Agent inspector title yields space to its actions", () => {
  assert.match(
    styles,
    /\.agent-template-panel--rail > \.ot-panel > \.ot-panel__header > \.ot-panel__title \{[^}]*flex: 1 1 auto;[^}]*min-inline-size: 0;[^}]*text-overflow: ellipsis;[^}]*white-space: nowrap;[^}]*\}/s,
  );
});

test("string Panel titles expose their full text on hover", () => {
  assert.match(
    panelSource,
    /title=\{typeof title === "string" \? title : undefined\}/,
  );
});
