const path = require("node:path");
const { runGrader } = require("../../grader-kit.cjs");

const workspace = path.resolve(process.argv[2] || "");
const phase = process.argv[3] || "full";
const seed = path.join(__dirname, "seed");

runGrader({
  workspace,
  phase,
  seed,
  modulePath: "src/retry.js",
  exports: ["validateJobs", "planRetries", "summarizeRetries"],
  libraryChecks: [
    ["job-validation", ({ assert, subject }) => {
      assert.ok(subject, "module did not load");
      const base = { id: "a", attempt: 0, maxAttempts: 2, baseDelayMs: 10, lastFailureAt: "2026-01-01T00:00:00Z" };
      assert.throws(() => subject.validateJobs([base, { ...base }]), /duplicate|unique/i);
      for (const patch of [{ attempt: -1 }, { maxAttempts: 0 }, { baseDelayMs: 0 }, { lastFailureAt: "nope" }]) {
        assert.throws(() => subject.validateJobs([{ ...base, ...patch }]));
      }
    }],
    ["backoff-cap-and-ordering", ({ assert, subject }) => {
      const jobs = [
        { id: "b", attempt: 20, maxAttempts: 30, baseDelayMs: 100000, lastFailureAt: "2026-01-01T00:00:00Z" },
        { id: "a", attempt: 1, maxAttempts: 3, baseDelayMs: 500, lastFailureAt: "2026-01-01T00:00:00Z" },
        { id: "c", attempt: 3, maxAttempts: 3, baseDelayMs: 1, lastFailureAt: "2026-01-01T00:00:00Z" },
      ];
      const before = JSON.stringify(jobs);
      const plan = subject.planRetries(jobs, new Date("2026-01-01T00:00:02Z"));
      assert.equal(JSON.stringify(jobs), before, "input was mutated");
      assert.deepEqual(plan.jobs.map((job) => [job.id, job.state, job.delayMs, job.nextAttemptAt]), [
        ["a", "ready", 1000, "2026-01-01T00:00:01.000Z"],
        ["b", "waiting", 3600000, "2026-01-01T01:00:00.000Z"],
        ["c", "exhausted", null, null],
      ]);
    }],
    ["retry-summary-and-now-validation", ({ assert, subject }) => {
      assert.throws(() => subject.planRetries([], "invalid"));
      const plan = subject.planRetries([
        { id: "a", attempt: 0, maxAttempts: 2, baseDelayMs: 1000, lastFailureAt: "2026-01-01T00:00:00Z" },
        { id: "b", attempt: 0, maxAttempts: 2, baseDelayMs: 2000, lastFailureAt: "2026-01-01T00:00:00Z" },
      ], "2025-12-31T23:59:59Z");
      assert.deepEqual(subject.summarizeRetries(plan), {
        jobs: 2, ready: 0, waiting: 2, exhausted: 0,
        nextWakeAt: "2026-01-01T00:00:01.000Z",
      });
    }],
  ],
  fullChecks: [
    ["cli-success-contract", ({ assert, fs, os, path, spawnSync, workspace }) => {
      const root = fs.mkdtempSync(path.join(os.tmpdir(), "retry-eval-"));
      try {
        const input = path.join(root, "input.json");
        const output = path.join(root, "output.json");
        fs.writeFileSync(input, JSON.stringify({ jobs: [{ id: "a", attempt: 0, maxAttempts: 2, baseDelayMs: 1000, lastFailureAt: "2026-01-01T00:00:00Z" }] }));
        const run = spawnSync(process.execPath, [path.join(workspace, "src", "cli.js"), "--input", input, "--now", "2026-01-01T00:00:02Z", "--output", output], { encoding: "utf8", cwd: workspace });
        assert.equal(run.status, 0, run.stderr);
        assert.equal(run.stdout, "Scheduled 1 jobs: 1 ready, 0 waiting, 0 exhausted.\n");
        assert.ok(fs.readFileSync(output, "utf8").endsWith("\n"));
      } finally { fs.rmSync(root, { recursive: true, force: true }); }
    }],
    ["cli-failure-contract", ({ assert, fs, os, path, spawnSync, workspace }) => {
      const root = fs.mkdtempSync(path.join(os.tmpdir(), "retry-invalid-"));
      try {
        const input = path.join(root, "input.json");
        const output = path.join(root, "output.json");
        fs.writeFileSync(input, JSON.stringify({ jobs: [] }));
        const run = spawnSync(process.execPath, [path.join(workspace, "src", "cli.js"), "--input", input, "--now", "bad", "--output", output], { encoding: "utf8", cwd: workspace });
        assert.notEqual(run.status, 0);
        assert.ok(run.stderr.trim());
        assert.equal(fs.existsSync(output), false);
      } finally { fs.rmSync(root, { recursive: true, force: true }); }
    }],
  ],
}).catch((error) => {
  process.stderr.write(`${error.stack || error}\n`);
  process.exitCode = 2;
});
