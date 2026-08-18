import { readFile, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { compile } from "json-schema-to-typescript";

const desktopRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const generatedRoot = join(desktopRoot, "src", "api", "generated");
const check = process.argv.includes("--check");
const contracts = [
  ["agent-event-envelope-v1", "AgentEventEnvelopeV1"],
  ["agent-activity-envelope-v1", "AgentActivityEnvelopeV1"],
  ["terminal-event-envelope-v1", "TerminalEventEnvelopeV1"],
  ["runtime-snapshot-v1", "RuntimeSnapshotV1"],
  ["desktop-http-v1", "DesktopHttpResponsesV1"],
];

const stale = [];
for (const [fileStem, typeName] of contracts) {
  const schemaPath = join(generatedRoot, `${fileStem}.schema.json`);
  const outputPath = join(generatedRoot, `${fileStem}.generated.ts`);
  const schema = JSON.parse(await readFile(schemaPath, "utf8"));
  schema.title = typeName;
  const generated = await compile(schema, typeName, {
    bannerComment:
      "/* eslint-disable */\n" +
      "// Generated from the Rust DTO schema. Run `pnpm contracts:generate`; do not edit.",
    declareExternallyReferenced: true,
    enableConstEnums: false,
    style: {
      bracketSpacing: true,
      printWidth: 100,
      semi: true,
      singleQuote: false,
      tabWidth: 2,
      trailingComma: "all",
      useTabs: false,
    },
  });

  if (check) {
    const existing = await readFile(outputPath, "utf8").catch(() => undefined);
    if (existing !== generated) {
      stale.push(outputPath);
    }
  } else {
    await writeFile(outputPath, generated, "utf8");
  }
}

if (stale.length > 0) {
  throw new Error(
    `generated TypeScript contracts are stale: ${stale.join(", ")}`,
  );
}
