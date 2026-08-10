const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");
const http = require("node:http");
const os = require("node:os");
const path = require("node:path");
const { pathToFileURL } = require("node:url");
const { spawn, spawnSync } = require("node:child_process");

function hash(filePath) {
  return crypto.createHash("sha256").update(fs.readFileSync(filePath)).digest("hex");
}

async function importFresh(filePath) {
  return import(`${pathToFileURL(filePath).href}?eval=${Date.now()}-${Math.random()}`);
}

function temporaryDirectory(prefix) {
  return fs.mkdtempSync(path.join(os.tmpdir(), prefix));
}

async function request(url) {
  return new Promise((resolve, reject) => {
    const call = http.get(url, (response) => {
      let body = "";
      response.setEncoding("utf8");
      response.on("data", (chunk) => { body += chunk; });
      response.on("end", () => resolve({ status: response.statusCode, headers: response.headers, body }));
    });
    call.on("error", reject);
  });
}

async function runChecks({ workspace, seed, phase = "full", checks, protectedPaths = ["ISSUE.md", "test"] }) {
  if (!workspace || !fs.existsSync(workspace)) {
    process.stderr.write("usage: grader.cjs <workspace> [phase]\n");
    process.exitCode = 2;
    return;
  }
  const results = [];
  const check = async (id, action) => {
    try {
      await action({ assert, crypto, fs, http, importFresh, os, path, request, spawn, spawnSync, workspace });
      results.push({ id, passed: true });
    } catch (error) {
      results.push({ id, passed: false, detail: String(error?.stack || error).slice(0, 1200) });
    }
  };
  for (const entry of checks) {
    if (entry.phase && entry.phase !== phase && phase !== "full") continue;
    await check(entry.id, entry.run);
  }
  await check("protected-files-unchanged", async ({ fs, path }) => {
    for (const relative of protectedPaths) {
      const before = path.join(seed, relative);
      const after = path.join(workspace, relative);
      const beforeStats = fs.statSync(before);
      if (beforeStats.isDirectory()) {
        const walk = (root) => fs.readdirSync(root, { withFileTypes: true }).flatMap((entry) => {
          const absolute = path.join(root, entry.name);
          return entry.isDirectory() ? walk(absolute) : [absolute];
        });
        for (const file of walk(before)) {
          const nested = path.relative(seed, file);
          assert.equal(hash(path.join(workspace, nested)), hash(file), `${nested} was modified`);
        }
      } else {
        assert.equal(hash(after), hash(before), `${relative} was modified`);
      }
    }
  });
  const passed = results.filter((entry) => entry.passed).length;
  const output = { schemaVersion: 1, phase, passed: passed === results.length, passedChecks: passed, totalChecks: results.length, checks: results };
  process.stdout.write(`${JSON.stringify(output, null, 2)}\n`);
  process.exitCode = output.passed ? 0 : 1;
}

module.exports = { runChecks, temporaryDirectory };
