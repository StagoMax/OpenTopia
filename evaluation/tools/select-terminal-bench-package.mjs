#!/usr/bin/env node

import { createHash } from "node:crypto";
import { mkdir, readdir, readFile, writeFile } from "node:fs/promises";
import { dirname, relative, resolve, sep } from "node:path";

function optionValue(argv, name, fallback = "") {
  const index = argv.indexOf(name);
  return index >= 0 && argv[index + 1] ? argv[index + 1] : fallback;
}

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function normalizedRelative(root, path) {
  return relative(root, path).split(sep).join("/");
}

async function filesRecursively(root) {
  const result = [];
  async function visit(directory) {
    const entries = await readdir(directory, { withFileTypes: true });
    for (const entry of entries) {
      const fullPath = resolve(directory, entry.name);
      if (entry.isDirectory()) await visit(fullPath);
      else if (entry.isFile()) result.push(fullPath);
    }
  }
  await visit(root);
  return result.sort((left, right) =>
    normalizedRelative(root, left).localeCompare(normalizedRelative(root, right)),
  );
}

async function treeSha256(root) {
  const digest = createHash("sha256");
  for (const file of await filesRecursively(root)) {
    digest.update(normalizedRelative(root, file));
    digest.update("\0");
    digest.update(await readFile(file));
    digest.update("\0");
  }
  return digest.digest("hex");
}

const output = optionValue(process.argv, "--output");
const packageDir = optionValue(process.argv, "--package-dir");
const count = Number(optionValue(process.argv, "--count", "40"));
const seed = optionValue(process.argv, "--seed", "opentopia-before-after-v1");
const dataset = optionValue(
  process.argv,
  "--dataset",
  "terminal-bench/terminal-bench@latest",
);

if (!output || !packageDir || !Number.isInteger(count) || count < 1) {
  throw new Error(
    "usage: select-terminal-bench-package.mjs --package-dir <downloaded terminal-bench directory> --output <json> [--count 40] [--seed value] [--dataset name@version]",
  );
}

const root = resolve(packageDir);
const directories = await readdir(root, { withFileTypes: true });
const tasks = [];
for (const entry of directories) {
  if (!entry.isDirectory()) continue;
  const taskTomlPath = resolve(root, entry.name, "task.toml");
  try {
    const contents = await readFile(taskTomlPath);
    tasks.push({
      taskId: entry.name,
      taskTomlPath: `${entry.name}/task.toml`,
      taskTomlSha256: sha256(contents),
    });
  } catch (error) {
    if (error?.code !== "ENOENT") throw error;
  }
}

tasks.sort((left, right) =>
  sha256(`${seed}:${left.taskId}`).localeCompare(sha256(`${seed}:${right.taskId}`)),
);
if (tasks.length < count) {
  throw new Error(`only ${tasks.length} Terminal-Bench tasks available; requested ${count}`);
}

const selected = tasks.slice(0, count);
const result = {
  schemaVersion: 2,
  dataset,
  source: "official Harbor package downloaded locally before selection",
  packageTreeSha256: await treeSha256(root),
  availableTaskCount: tasks.length,
  selectionMethod:
    "SHA-256(seed:taskId) rank over immediate task directories containing task.toml in the pinned local package",
  seed,
  selectedCount: selected.length,
  selectionFingerprintSha256: sha256(selected.map((task) => task.taskId).join("\n")),
  tasks: selected,
};

const outputPath = resolve(output);
await mkdir(dirname(outputPath), { recursive: true });
await writeFile(outputPath, `${JSON.stringify(result, null, 2)}\n`, "utf8");
console.log(`TERMINAL_BENCH_SELECTION=${outputPath}`);
console.log(`PACKAGE_TREE_SHA256=${result.packageTreeSha256}`);
console.log(`SELECTION_FINGERPRINT=${result.selectionFingerprintSha256}`);
