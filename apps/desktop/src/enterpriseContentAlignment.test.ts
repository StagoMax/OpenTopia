import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const enterpriseStyles = readFileSync(
  new URL("./components/enterprise/enterprise.css", import.meta.url),
  "utf8",
);
const connectionStyles = readFileSync(
  new URL("./styles/connections.css", import.meta.url),
  "utf8",
);

test("Trust detail content stays aligned to the workspace start edge", () => {
  assert.match(
    enterpriseStyles,
    /\.enterprise-trust\.enterprise-core-detail \{[^}]*inline-size: 100%;[^}]*max-inline-size: calc\(var\(--space-16\) \* 30\);[^}]*margin-inline: var\(--space-0\);[^}]*padding: var\(--space-8\);[^}]*\}/s,
  );
});

test("Connection detail content stays aligned to the workspace start edge", () => {
  assert.match(
    connectionStyles,
    /\.connection-details--core \{[^}]*inline-size: 100%;[^}]*max-inline-size: calc\(var\(--space-16\) \* 30\);[^}]*margin-inline: var\(--space-0\);[^}]*padding: var\(--space-8\);[^}]*\}/s,
  );
  assert.match(
    connectionStyles,
    /\.connection-details--core \.connection-operation-summary,[^}]*\.connection-details--core \.connection-operation-facts \{[^}]*max-inline-size: none;[^}]*margin-inline: var\(--space-0\);[^}]*\}/s,
  );
});
