import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const sidebarStyles = readFileSync(
  new URL("./styles/app-goal-conversation.css", import.meta.url),
  "utf8",
);

test("keeps the processing indicator visible on the selected task row", () => {
  const selectedProcessingStyles = sidebarStyles.slice(
    sidebarStyles.indexOf(
      ".thread-row-wrap.active.is-processing:is(:hover, :focus-within)",
    ),
    sidebarStyles.indexOf(".thread-row-wrap:focus-within .thread-row-more"),
  );
  assert.ok(
    selectedProcessingStyles.startsWith(
      ".thread-row-wrap.active.is-processing:is(:hover, :focus-within)",
    ),
  );
  assert.match(selectedProcessingStyles, /grid-template-columns:/);
  assert.match(selectedProcessingStyles, /visibility:\s*visible/);
  assert.match(selectedProcessingStyles, /grid-column:\s*3/);
});
