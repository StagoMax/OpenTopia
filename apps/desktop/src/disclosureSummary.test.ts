import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const componentSource = readFileSync(
  new URL("./components/ui/DisclosureSummary.tsx", import.meta.url),
  "utf8",
);
const styles = readFileSync(
  new URL("./styles/ui.css", import.meta.url),
  "utf8",
);

test("disclosure summaries expose visible collapsed and expanded hints", () => {
  assert.match(componentSource, />展开<\/span>/);
  assert.match(componentSource, />收起<\/span>/);
  assert.match(componentSource, /className="ot-disclosure-summary__chevron"/);
  assert.match(
    styles,
    /details\[open\] > \.ot-disclosure-summary \.ot-disclosure-summary__state-collapsed \{[^}]*display: none;/s,
  );
  assert.match(
    styles,
    /details\[open\] > \.ot-disclosure-summary \.ot-disclosure-summary__state-expanded \{[^}]*display: inline;/s,
  );
});

test("disclosure chevrons communicate the open state", () => {
  assert.match(
    styles,
    /details\[open\] > \.ot-disclosure-summary \.ot-disclosure-summary__chevron \{[^}]*transform: rotate\(90deg\);/s,
  );
});
