import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const styles = readFileSync(
  new URL("./components/UsageLogDashboard.css", import.meta.url),
  "utf8",
);

test("usage log constrains its grid column to the available tool-stage width", () => {
  assert.match(
    styles,
    /\.usage-log-dashboard \{[^}]*min-width: 0;[^}]*grid-template-columns: minmax\(0, 1fr\);[^}]*grid-template-rows: auto minmax\(0, 1fr\);[^}]*overflow: hidden;[^}]*\}/s,
  );
  assert.match(
    styles,
    /\.usage-log-scroll \{[^}]*min-width: 0;[^}]*overflow: auto;[^}]*\}/s,
  );
});

test("long task titles can shrink without widening the usage dashboard", () => {
  assert.match(styles, /\.usage-log-header \{[^}]*min-width: 0;[^}]*\}/s);
  assert.match(
    styles,
    /\.usage-log-heading \{[^}]*flex: 1 1 auto;[^}]*min-width: 0;[^}]*\}/s,
  );
  assert.match(
    styles,
    /\.usage-log-heading > div \{[^}]*min-width: 0;[^}]*\}/s,
  );
});
