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
  let planner;
  await check("module-loads", async () => {
    planner = await import(
      `${pathToFileURL(path.join(workspace, "src", "planner.js")).href}?eval=${Date.now()}`
    );
    for (const name of ["parseWorkspace", "buildReleaseWaves", "renderReleasePlan"]) {
      assert.equal(typeof planner[name], "function", `${name} must be exported`);
    }
  });

  const graph = {
    packages: [
      { name: "deploy", dependencies: ["web", "worker"] },
      { name: "shared", dependencies: [] },
      { name: "worker", dependencies: ["shared"] },
      { name: "web", dependencies: ["api", "shared"] },
      { name: "api", dependencies: ["shared"] },
      { name: "docs", dependencies: [] },
    ],
  };
  await check("strict-parse-and-sort", () => {
    const parsed = planner.parseWorkspace(JSON.stringify(graph));
    assert.deepEqual(
      parsed.map((item) => item.name),
      ["api", "deploy", "docs", "shared", "web", "worker"],
    );
    assert.deepEqual(parsed.find((item) => item.name === "web").dependencies, ["api", "shared"]);
    for (const invalid of [
      { packages: [{ name: "Bad", dependencies: [] }] },
      { packages: [{ name: "app", dependencies: ["app"] }] },
      { packages: [{ name: "a", dependencies: [] }, { name: "a", dependencies: [] }] },
      { packages: [{ name: "a", dependencies: ["missing"] }] },
      { packages: [{ name: "a", dependencies: [], extra: true }] },
      { packages: [], extra: true },
    ]) {
      assert.throws(() => planner.parseWorkspace(JSON.stringify(invalid)));
    }
  });

  await check("parallel-wave-contract", () => {
    const packages = planner.parseWorkspace(JSON.stringify(graph));
    assert.deepEqual(planner.buildReleaseWaves(packages), [
      ["docs", "shared"],
      ["api", "worker"],
      ["web"],
      ["deploy"],
    ]);
  });

  await check("cycle-detection", () => {
    assert.throws(
      () =>
        planner.buildReleaseWaves([
          { name: "a", dependencies: ["c"] },
          { name: "b", dependencies: ["a"] },
          { name: "c", dependencies: ["b"] },
        ]),
      /cycle/i,
    );
  });

  await check("render-contract", () => {
    const packages = planner.parseWorkspace(JSON.stringify(graph));
    assert.deepEqual(planner.renderReleasePlan(packages), {
      packageCount: 6,
      waveCount: 4,
      waves: [
        { index: 1, packages: ["docs", "shared"] },
        { index: 2, packages: ["api", "worker"] },
        { index: 3, packages: ["web"] },
        { index: 4, packages: ["deploy"] },
      ],
    });
  });

  await check("protected-files-unchanged", () => {
    for (const relative of ["SPEC.md", path.join("test", "contract.test.js")]) {
      assert.equal(hash(path.join(workspace, relative)), hash(path.join(seed, relative)));
    }
  });

  if (phase === "full") {
    const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "opentopia-deps-eval-"));
    try {
      await check("cli-success-contract", () => {
        const input = path.join(tempRoot, "workspace.json");
        const output = path.join(tempRoot, "plan.json");
        fs.writeFileSync(input, JSON.stringify(graph));
        const run = spawnSync(
          process.execPath,
          [path.join(workspace, "src", "cli.js"), "--input", input, "--output", output],
          { cwd: workspace, encoding: "utf8" },
        );
        assert.equal(run.status, 0, run.stderr);
        assert.equal(run.stdout, "Planned 6 packages in 4 waves.\n");
        const raw = fs.readFileSync(output, "utf8");
        assert.ok(raw.endsWith("\n"));
        assert.equal(JSON.parse(raw).waveCount, 4);
      });
      await check("cli-failure-contract", () => {
        const input = path.join(tempRoot, "cycle.json");
        const output = path.join(tempRoot, "cycle-plan.json");
        fs.writeFileSync(
          input,
          JSON.stringify({
            packages: [
              { name: "a", dependencies: ["b"] },
              { name: "b", dependencies: ["a"] },
            ],
          }),
        );
        const run = spawnSync(
          process.execPath,
          [path.join(workspace, "src", "cli.js"), "--input", input, "--output", output],
          { cwd: workspace, encoding: "utf8" },
        );
        assert.notEqual(run.status, 0);
        assert.match(run.stderr, /cycle/i);
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
