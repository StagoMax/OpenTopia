const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { pathToFileURL } = require("node:url");
const { spawnSync } = require("node:child_process");

const workspace = path.resolve(process.argv[2] || "");
const phase = process.argv[3] || "full";
const seed = path.join(__dirname, "seed");
if (!workspace || !["library", "full"].includes(phase)) {
  process.stderr.write("usage: grader.cjs <workspace> <library|full>\n");
  process.exit(2);
}

const checks = [];
async function check(id, action) {
  try {
    await action();
    checks.push({ id, passed: true });
  } catch (error) {
    checks.push({ id, passed: false, detail: String(error?.message || error).slice(0, 300) });
  }
}
function hash(filePath) {
  return crypto.createHash("sha256").update(fs.readFileSync(filePath)).digest("hex");
}

async function main() {
  let config;
  await check("module-loads", async () => {
    config = await import(
      `${pathToFileURL(path.join(workspace, "src", "config.js")).href}?eval=${Date.now()}`
    );
    for (const name of ["parseLegacyConfig", "applyOverrides", "serializeConfig"]) {
      assert.equal(typeof config[name], "function", `${name} must be exported`);
    }
  });

  const validLegacy = {
    version: 1,
    endpoint: "http://localhost:8080/v1///",
    model: " model-z ",
    timeout_seconds: 12,
    features: ["tools", "stream"],
  };
  await check("legacy-normalization", () => {
    const result = config.parseLegacyConfig(JSON.stringify(validLegacy));
    assert.deepEqual(result, {
      schemaVersion: 2,
      provider: { baseUrl: "http://localhost:8080/v1", model: "model-z" },
      timeoutMs: 12000,
      capabilities: { stream: true, tools: true },
    });
  });

  await check("strict-validation", () => {
    const variants = [
      { ...validLegacy, version: 2 },
      { ...validLegacy, endpoint: "https://user:pass@example.test/v1" },
      { ...validLegacy, timeout_seconds: 0 },
      { ...validLegacy, timeout_seconds: 301 },
      { ...validLegacy, features: ["stream", "stream"] },
      { ...validLegacy, features: ["unknown"] },
      { ...validLegacy, extra: true },
    ];
    for (const value of variants) {
      assert.throws(() => config.parseLegacyConfig(JSON.stringify(value)));
    }
    assert.throws(() => config.parseLegacyConfig("{"));
  });

  await check("override-contract", () => {
    const original = config.parseLegacyConfig(JSON.stringify(validLegacy));
    const snapshot = JSON.stringify(original);
    const result = config.applyOverrides(original, {
      APP_BASE_URL: "https://api.example.test/v2/",
      APP_MODEL: " next-model ",
      APP_TIMEOUT_MS: "90000",
      OTHER: "ignored",
    });
    assert.equal(JSON.stringify(original), snapshot, "config was mutated");
    assert.deepEqual(result.provider, {
      baseUrl: "https://api.example.test/v2",
      model: "next-model",
    });
    assert.equal(result.timeoutMs, 90000);
    assert.throws(() => config.applyOverrides(original, { APP_TIMEOUT_MS: "1.5" }));
    assert.throws(() => config.applyOverrides(original, { APP_BASE_URL: "file:///x" }));
  });

  await check("serialization-contract", () => {
    const normalized = config.parseLegacyConfig(JSON.stringify(validLegacy));
    const serialized = config.serializeConfig(normalized);
    assert.ok(serialized.endsWith("\n"));
    assert.equal(serialized.endsWith("\n\n"), false);
    assert.equal(
      serialized,
      `${JSON.stringify(normalized, null, 2)}\n`,
      "serialization must be deterministic",
    );
  });

  await check("protected-files-unchanged", () => {
    for (const relative of ["SPEC.md", path.join("test", "contract.test.js")]) {
      assert.equal(hash(path.join(workspace, relative)), hash(path.join(seed, relative)));
    }
  });

  if (phase === "full") {
    const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "opentopia-config-eval-"));
    try {
      await check("cli-success-contract", () => {
        const input = path.join(tempRoot, "legacy.json");
        const output = path.join(tempRoot, "normalized.json");
        fs.writeFileSync(input, JSON.stringify(validLegacy));
        const run = spawnSync(
          process.execPath,
          [path.join(workspace, "src", "cli.js"), "--input", input, "--output", output],
          {
            cwd: workspace,
            encoding: "utf8",
            env: { ...process.env, APP_MODEL: "cli-model", APP_TIMEOUT_MS: "15000" },
          },
        );
        assert.equal(run.status, 0, run.stderr);
        assert.equal(run.stdout, "Migrated config for cli-model.\n");
        const result = JSON.parse(fs.readFileSync(output, "utf8"));
        assert.equal(result.provider.model, "cli-model");
        assert.equal(result.timeoutMs, 15000);
      });
      await check("cli-failure-contract", () => {
        const input = path.join(tempRoot, "invalid.json");
        const output = path.join(tempRoot, "invalid-output.json");
        fs.writeFileSync(input, "{}");
        const run = spawnSync(
          process.execPath,
          [path.join(workspace, "src", "cli.js"), "--input", input, "--output", output],
          { cwd: workspace, encoding: "utf8" },
        );
        assert.notEqual(run.status, 0);
        assert.ok(run.stderr.trim());
        assert.equal(fs.existsSync(output), false);
      });
    } finally {
      fs.rmSync(tempRoot, { recursive: true, force: true });
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

main().catch((error) => {
  process.stdout.write(
    `${JSON.stringify({
      schemaVersion: 1,
      phase,
      passed: false,
      passedChecks: 0,
      totalChecks: 1,
      checks: [{ id: "grader-internal", passed: false, detail: error.message }],
    })}\n`,
  );
  process.exitCode = 1;
});
