const path = require("node:path");
const { runGrader } = require("../../grader-kit.cjs");

const workspace = path.resolve(process.argv[2] || "");
const phase = process.argv[3] || "full";
const seed = path.join(__dirname, "seed");

runGrader({
  workspace,
  phase,
  seed,
  modulePath: "src/digest.js",
  exports: ["validateEvents", "buildDigests", "summarizeDigests"],
  libraryChecks: [
    ["event-validation", ({ assert, subject }) => {
      assert.ok(subject, "module did not load");
      const base = { id: "e", recipient: "u", category: "c", severity: "info", createdAt: "2026-01-01T00:00:00Z", read: false };
      assert.throws(() => subject.validateEvents([base, { ...base }]), /duplicate|unique/i);
      for (const patch of [{ recipient: "" }, { category: "" }, { severity: "urgent" }, { createdAt: "bad" }, { read: 0 }]) {
        assert.throws(() => subject.validateEvents([{ ...base, ...patch }]));
      }
    }],
    ["digest-filter-and-stable-order", ({ assert, subject }) => {
      const events = [
        { id: "z", recipient: "u", category: "a", severity: "warning", createdAt: "2026-01-01T00:00:02Z", read: false },
        { id: "b", recipient: "u", category: "b", severity: "critical", createdAt: "2026-01-01T00:00:03Z", read: false },
        { id: "a", recipient: "u", category: "a", severity: "critical", createdAt: "2026-01-01T00:00:03Z", read: false },
        { id: "old", recipient: "u", category: "a", severity: "critical", createdAt: "2025-12-31T23:59:59Z", read: false },
        { id: "read", recipient: "v", category: "a", severity: "critical", createdAt: "2026-01-01T00:00:00Z", read: true },
      ];
      const before = JSON.stringify(events);
      const result = subject.buildDigests(events, "2026-01-01T00:00:00Z");
      assert.equal(JSON.stringify(events), before, "input was mutated");
      assert.deepEqual(result.digests[0].items.map((item) => item.id), ["a", "b", "z"]);
      assert.deepEqual(result.digests[0].critical, 2);
      assert.deepEqual(result.digests[0].warning, 1);
    }],
    ["digest-summary-and-since-validation", ({ assert, subject }) => {
      assert.throws(() => subject.buildDigests([], "invalid"));
      const empty = subject.buildDigests([], "2026-01-01T00:00:00Z");
      assert.deepEqual(empty, { since: "2026-01-01T00:00:00.000Z", digests: [] });
      assert.deepEqual(subject.summarizeDigests(empty), { recipients: 0, notifications: 0, critical: 0, warning: 0, info: 0 });
    }],
  ],
  fullChecks: [
    ["cli-success-contract", ({ assert, fs, os, path, spawnSync, workspace }) => {
      const root = fs.mkdtempSync(path.join(os.tmpdir(), "digest-eval-"));
      try {
        const input = path.join(root, "input.json");
        const output = path.join(root, "output.json");
        fs.writeFileSync(input, JSON.stringify({ events: [{ id: "1", recipient: "u", category: "build", severity: "warning", createdAt: "2026-01-01T00:00:00Z", read: false }] }));
        const run = spawnSync(process.execPath, [path.join(workspace, "src", "cli.js"), "--input", input, "--since", "2026-01-01T00:00:00Z", "--output", output], { encoding: "utf8", cwd: workspace });
        assert.equal(run.status, 0, run.stderr);
        assert.equal(run.stdout, "Built 1 digests with 1 unread notifications.\n");
        assert.ok(fs.readFileSync(output, "utf8").endsWith("\n"));
      } finally { fs.rmSync(root, { recursive: true, force: true }); }
    }],
    ["cli-failure-contract", ({ assert, fs, os, path, spawnSync, workspace }) => {
      const root = fs.mkdtempSync(path.join(os.tmpdir(), "digest-invalid-"));
      try {
        const input = path.join(root, "input.json");
        const output = path.join(root, "output.json");
        fs.writeFileSync(input, JSON.stringify({ events: [] }));
        const run = spawnSync(process.execPath, [path.join(workspace, "src", "cli.js"), "--input", input, "--since", "bad", "--output", output], { encoding: "utf8", cwd: workspace });
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
