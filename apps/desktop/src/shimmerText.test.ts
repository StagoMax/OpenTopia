import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const styles = await readFile(
  new URL("./styles/ui.css", import.meta.url),
  "utf8",
);

test("shimmer uses a masked transform sweep instead of an opacity pulse", () => {
  const sweepRule = styles.match(/\.ot-shimmer-text__sweep\s*\{([^}]*)\}/)?.[1];
  const highlightRule = [
    ...styles.matchAll(/\.ot-shimmer-text__highlight\s*\{([^}]*)\}/g),
  ]
    .map((match) => match[1])
    .find((rule) => rule.includes("ot-shimmer-counter-sweep"));

  assert.ok(sweepRule);
  assert.match(sweepRule, /mask-image:\s*linear-gradient/);
  assert.match(sweepRule, /will-change:\s*transform/);
  assert.match(sweepRule, /animation:\s*ot-shimmer-sweep/);
  assert.doesNotMatch(sweepRule, /background-position|opacity/);

  assert.ok(highlightRule);
  assert.match(highlightRule, /animation:\s*ot-shimmer-counter-sweep/);
  assert.match(
    styles,
    /@keyframes ot-shimmer-sweep\s*\{[\s\S]*?translate3d\(100%, 0, 0\)/,
  );
  assert.match(
    styles,
    /@keyframes ot-shimmer-counter-sweep\s*\{[\s\S]*?translate3d\(-100%, 0, 0\)/,
  );
});

test("shimmer is hidden when reduced motion is requested", () => {
  assert.match(
    styles,
    /@media \(prefers-reduced-motion: reduce\)[\s\S]*?\.ot-shimmer-text__sweep\s*\{\s*display:\s*none/,
  );
  assert.match(
    styles,
    /\[data-reduce-motion="on"\] \.ot-shimmer-text__sweep\s*\{\s*display:\s*none/,
  );
});
