import { readFile } from "node:fs/promises";
import { glob } from "node:fs/promises";
import { resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const uiRoot = resolve(root, "apps/desktop/src/components/ui");
const uiStyles = resolve(root, "apps/desktop/src/styles/ui.css");
const tokens = resolve(root, "apps/desktop/src/styles/tokens.css");
const master = resolve(root, "design-system/MASTER.md");
const agentInstructions = resolve(root, "AGENTS.md");
const violations = [];

for (const required of [tokens, master, agentInstructions]) {
  try {
    await readFile(required, "utf8");
  } catch {
    violations.push(`Missing required design-system file: ${required}`);
  }
}

for await (const file of glob("**/*.{ts,tsx,css}", { cwd: uiRoot })) {
  const absolutePath = resolve(uiRoot, file);
  const source = await readFile(absolutePath, "utf8");
  if (/#(?:[0-9a-fA-F]{3,8})\b/.test(source)) {
    violations.push(`${absolutePath}: raw colors belong in styles/tokens.css`);
  }
}

const sharedStyles = await readFile(uiStyles, "utf8");
if (/#(?:[0-9a-fA-F]{3,8})\b/.test(sharedStyles)) {
  violations.push(`${uiStyles}: raw colors belong in styles/tokens.css`);
}

if (violations.length > 0) {
  console.error("Design system check failed:\n" + violations.join("\n"));
  process.exitCode = 1;
} else {
  console.log("Design system check passed.");
}
