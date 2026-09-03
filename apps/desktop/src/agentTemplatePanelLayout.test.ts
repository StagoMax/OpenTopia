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

test("Flow Agent inspector header keeps actions inset and can wrap", () => {
  assert.match(
    styles,
    /\.agent-template-panel--rail > \.ot-panel > \.ot-panel__header \{[^}]*flex-wrap: wrap;[^}]*min-block-size: calc\(var\(--control-height-lg\) \+ var\(--space-4\)\);[^}]*padding: var\(--space-3\) var\(--space-4\);[^}]*\}/s,
  );
  assert.match(
    styles,
    /\.agent-template-panel--rail \.agent-template-panel__header-actions \{[^}]*flex: 0 0 auto;[^}]*max-inline-size: 100%;[^}]*margin-inline-start: auto;[^}]*flex-wrap: wrap;[^}]*gap: var\(--space-2\);[^}]*\}/s,
  );
});

test("Flow Agent inspector title reserves a useful width before wrapping", () => {
  assert.match(
    styles,
    /\.agent-template-panel--rail > \.ot-panel > \.ot-panel__header > \.ot-panel__title \{[^}]*flex: 1 1 calc\(var\(--space-16\) \* 4\);[^}]*min-inline-size: 0;[^}]*text-overflow: ellipsis;[^}]*white-space: nowrap;[^}]*\}/s,
  );
});

test("string Panel titles expose their full text on hover", () => {
  assert.match(
    panelSource,
    /title=\{typeof title === "string" \? title : undefined\}/,
  );
});
