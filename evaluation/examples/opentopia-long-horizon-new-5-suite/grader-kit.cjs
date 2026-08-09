const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { pathToFileURL } = require("node:url");
const { spawnSync } = require("node:child_process");

function hash(filePath) {
  return crypto.createHash("sha256").update(fs.readFileSync(filePath)).digest("hex");
}

async function runGrader({ workspace, phase, seed, modulePath, exports, libraryChecks, fullChecks }) {
  if (!workspace || !["library", "full"].includes(phase)) {
    process.stderr.write("usage: grader.cjs <workspace> <library|full>\n");
    process.exitCode = 2;
    return;
  }

  const checks = [];
  async function check(id, action) {
    try {
      await action();
      checks.push({ id, passed: true });
    } catch (error) {
      checks.push({ id, passed: false, detail: String(error?.message || error).slice(0, 500) });
    }
  }

  let subject;
  await check("module-loads", async () => {
    subject = await import(`${pathToFileURL(path.join(workspace, modulePath)).href}?eval=${Date.now()}`);
    for (const name of exports) {
      assert.equal(typeof subject[name], "function", `${name} must be exported`);
    }
  });

  const context = { assert, fs, os, path, spawnSync, subject, workspace };
  for (const [id, action] of libraryChecks) {
    await check(id, () => action(context));
  }

  await check("protected-files-unchanged", () => {
    for (const relative of ["SPEC.md", path.join("test", "contract.test.js")]) {
      assert.equal(hash(path.join(workspace, relative)), hash(path.join(seed, relative)), `${relative} was modified`);
    }
  });

  if (phase === "full") {
    for (const [id, action] of fullChecks) {
      await check(id, () => action(context));
    }
  }

  const passedChecks = checks.filter((item) => item.passed).length;
  const result = {
    schemaVersion: 1,
    phase,
    passed: passedChecks === checks.length,
    passedChecks,
    totalChecks: checks.length,
    checks,
  };
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
  process.exitCode = result.passed ? 0 : 1;
}

module.exports = { runGrader };
