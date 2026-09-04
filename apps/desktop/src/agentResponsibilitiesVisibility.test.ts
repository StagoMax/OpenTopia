import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const agentsPageSource = readFileSync(
  new URL("./components/enterprise/AgentsPage.tsx", import.meta.url),
  "utf8",
);

test("Agent responsibility instructions are expanded for each selection", () => {
  assert.match(
    agentsPageSource,
    /<details\s+className="enterprise-agent-instructions"\s+key=\{selectedTemplateKey \?\? undefined\}\s+open\s*>/s,
  );
});
