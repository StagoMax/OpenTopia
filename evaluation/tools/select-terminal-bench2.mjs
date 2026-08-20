#!/usr/bin/env node

import { createHash } from "node:crypto";
import { mkdir, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";

function optionValue(argv, name, fallback = "") {
  const index = argv.indexOf(name);
  return index >= 0 && argv[index + 1] ? argv[index + 1] : fallback;
}

function hash(value) {
  return createHash("sha256").update(value).digest("hex");
}

async function githubJson(url) {
  const response = await fetch(url, {
    headers: { "user-agent": "OpenTopia-evaluation-prep" },
  });
  if (!response.ok) throw new Error(`GitHub request failed: ${response.status} ${url}`);
  return response.json();
}

const output = optionValue(process.argv, "--output");
const count = Number(optionValue(process.argv, "--count", "40"));
const seed = optionValue(process.argv, "--seed", "opentopia-before-after-v1");
if (!output || !Number.isInteger(count) || count < 1) {
  throw new Error("usage: select-terminal-bench2.mjs --output <json> [--count 40] [--seed value]");
}
const repository = "harbor-framework/terminal-bench-2";
const commit = await githubJson(`https://api.github.com/repos/${repository}/commits/main`);
const tree = await githubJson(`https://api.github.com/repos/${repository}/git/trees/${commit.sha}?recursive=1`);
if (tree.truncated) throw new Error("GitHub returned a truncated tree; cannot make a complete selection");
const tasks = tree.tree
  .filter((entry) => entry.type === "blob" && /^[^/]+\/task\.toml$/.test(entry.path))
  .map((entry) => ({ taskId: entry.path.slice(0, -"/task.toml".length), taskTomlPath: entry.path, taskTomlBlobSha: entry.sha }))
  .sort((left, right) => hash(`${seed}:${left.taskId}`).localeCompare(hash(`${seed}:${right.taskId}`)));
if (tasks.length < count) throw new Error(`only ${tasks.length} Terminal-Bench tasks available; requested ${count}`);
const selected = tasks.slice(0, count);
const result = {
  schemaVersion: 1,
  dataset: "terminal-bench@2.0",
  sourceRepository: repository,
  sourceCommit: commit.sha,
  sourceCommitUrl: commit.html_url,
  treeSha: tree.sha,
  availableTaskCount: tasks.length,
  selectionMethod: "SHA-256(seed:taskId) rank over all task.toml entries at the pinned source commit",
  seed,
  selectedCount: selected.length,
  selectionFingerprintSha256: hash(selected.map((task) => task.taskId).join("\n")),
  tasks: selected,
};
const outputPath = resolve(output);
await mkdir(dirname(outputPath), { recursive: true });
await writeFile(outputPath, `${JSON.stringify(result, null, 2)}\n`, "utf8");
console.log(`TERMINAL_BENCH_SELECTION=${outputPath}`);
console.log(`SELECTION_FINGERPRINT=${result.selectionFingerprintSha256}`);
