import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const sidebarStyles = readFileSync(
  new URL("./styles/app-goal-conversation.css", import.meta.url),
  "utf8",
);

test("replaces the selected processing indicator with the task menu on hover", () => {
  const hoverStyles = sidebarStyles.slice(
    sidebarStyles.indexOf(".thread-row-wrap:hover .thread-row-status"),
    sidebarStyles.indexOf(".thread-row-wrap:focus-within .thread-row-more"),
  );

  assert.match(hoverStyles, /visibility:\s*hidden/);
  assert.match(hoverStyles, /grid-column:\s*2/);
  assert.doesNotMatch(
    sidebarStyles,
    /\.thread-row-wrap\.active\.is-processing:is\(:hover, :focus-within\)/,
  );
});
