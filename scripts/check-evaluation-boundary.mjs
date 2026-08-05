import { readFile, readdir } from "node:fs/promises";
import path from "node:path";
import process from "node:process";

const repoRoot = path.resolve(import.meta.dirname, "..");
const productRoots = ["apps", "crates"];
const sourceExtensions = new Set([".cjs", ".css", ".html", ".js", ".json", ".md", ".mjs", ".rs", ".ts", ".tsx"]);
const forbiddenProductPatterns = [
  ["EvaluationPanel", "desktop evaluation UI"],
  ["/api/evaluations", "product evaluation API"],
  ["OPENTOPIA_EVAL_", "evaluation-only environment variable"],
  ["opentopia-eval-runtime", "evaluation runtime descriptor"],
  ["evaluation_runs", "product evaluation storage"],
  ["EvaluationTaskResult", "product evaluation result model"],
  ["EvaluationRun", "product evaluation run model"]
];

async function filesUnder(relativeRoot) {
  const files = [];
  async function visit(directory) {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      if (["dist", "node_modules", "release", "target"].includes(entry.name)) continue;
      const entryPath = path.join(directory, entry.name);
      if (entry.isDirectory()) await visit(entryPath);
      if (entry.isFile() && sourceExtensions.has(path.extname(entry.name))) files.push(entryPath);
    }
  }
  await visit(path.join(repoRoot, relativeRoot));
  return files;
}

const violations = [];
for (const root of productRoots) {
  for (const filePath of await filesUnder(root)) {
    const content = await readFile(filePath, "utf8");
    for (const [pattern, label] of forbiddenProductPatterns) {
      if (content.includes(pattern)) {
        violations.push(`${path.relative(repoRoot, filePath)} contains ${label} (${pattern})`);
      }
    }
  }
}

for (const filePath of await filesUnder("evaluation/src")) {
  const content = await readFile(filePath, "utf8");
  if (/from\s+["'][^"']*(?:apps|crates)[\\/]/.test(content)) {
    violations.push(`${path.relative(repoRoot, filePath)} imports product source`);
  }
}

const desktopPackagePath = path.join(repoRoot, "apps", "desktop", "package.json");
const desktopPackage = JSON.parse(await readFile(desktopPackagePath, "utf8"));
const packagedPaths = JSON.stringify({
  files: desktopPackage.build?.files ?? [],
  extraResources: desktopPackage.build?.extraResources ?? []
});
if (/evaluation/i.test(packagedPaths)) {
  violations.push("apps/desktop/package.json packages evaluation files");
}

if (violations.length > 0) {
  process.stderr.write(`Evaluation boundary check failed:\n- ${violations.join("\n- ")}\n`);
  process.exitCode = 1;
} else {
  process.stdout.write("Evaluation boundary check passed.\n");
}
