const path = require("node:path");
const { runGrader } = require("../../grader-kit.cjs");

const workspace = path.resolve(process.argv[2] || "");
const phase = process.argv[3] || "full";
const seed = path.join(__dirname, "seed");

runGrader({
  workspace,
  phase,
  seed,
  modulePath: "src/access.js",
  exports: ["validateGrants", "auditAccess", "summarizeAudit"],
  libraryChecks: [
    ["grant-validation", ({ assert, subject }) => {
      assert.ok(subject, "module did not load");
      const base = { id: "g", user: "u", resource: "r", role: "viewer", source: "direct", expiresAt: null };
      assert.throws(() => subject.validateGrants([base, { ...base }]), /duplicate|unique/i);
      for (const patch of [{ user: "" }, { role: "owner" }, { source: "team" }, { expiresAt: "bad" }]) {
        assert.throws(() => subject.validateGrants([{ ...base, ...patch }]));
      }
    }],
    ["effective-tie-breaking-and-boundary", ({ assert, subject }) => {
      const grants = [
        { id: "z", user: "u", resource: "r", role: "admin", source: "group", expiresAt: null },
        { id: "b", user: "u", resource: "r", role: "admin", source: "direct", expiresAt: null },
        { id: "a", user: "u", resource: "r", role: "admin", source: "direct", expiresAt: null },
        { id: "expired", user: "a", resource: "x", role: "viewer", source: "group", expiresAt: "2026-01-01T00:00:00Z" },
      ];
      const before = JSON.stringify(grants);
      const audit = subject.auditAccess(grants, "2026-01-01T00:00:00Z");
      assert.equal(JSON.stringify(grants), before, "input was mutated");
      assert.deepEqual(audit.effective, [{ user: "u", resource: "r", role: "admin", source: "direct", grantId: "a" }]);
      assert.deepEqual(audit.expired, [{ grantId: "expired", user: "a", resource: "x", expiredAt: "2026-01-01T00:00:00.000Z" }]);
      assert.deepEqual(audit.shadowed.map((row) => [row.grantId, row.effectiveGrantId]), [["b", "a"], ["z", "a"]]);
    }],
    ["audit-summary-and-now-validation", ({ assert, subject }) => {
      assert.throws(() => subject.auditAccess([], "invalid"));
      const audit = subject.auditAccess([
        { id: "1", user: "a", resource: "x", role: "admin", source: "direct", expiresAt: null },
        { id: "2", user: "b", resource: "x", role: "viewer", source: "group", expiresAt: null },
      ], "2026-01-01T00:00:00Z");
      assert.deepEqual(subject.summarizeAudit(audit), { grants: 2, effective: 2, expired: 0, shadowed: 0, adminAccess: 1 });
    }],
  ],
  fullChecks: [
    ["cli-success-contract", ({ assert, fs, os, path, spawnSync, workspace }) => {
      const root = fs.mkdtempSync(path.join(os.tmpdir(), "access-eval-"));
      try {
        const input = path.join(root, "input.json");
        const output = path.join(root, "output.json");
        fs.writeFileSync(input, JSON.stringify({ grants: [{ id: "1", user: "a", resource: "x", role: "admin", source: "direct", expiresAt: null }] }));
        const run = spawnSync(process.execPath, [path.join(workspace, "src", "cli.js"), "--input", input, "--now", "2026-01-01T00:00:00Z", "--output", output], { encoding: "utf8", cwd: workspace });
        assert.equal(run.status, 0, run.stderr);
        assert.equal(run.stdout, "Audited 1 grants: 1 effective, 0 expired, 0 shadowed.\n");
        assert.ok(fs.readFileSync(output, "utf8").endsWith("\n"));
      } finally { fs.rmSync(root, { recursive: true, force: true }); }
    }],
    ["cli-failure-contract", ({ assert, fs, os, path, spawnSync, workspace }) => {
      const root = fs.mkdtempSync(path.join(os.tmpdir(), "access-invalid-"));
      try {
        const input = path.join(root, "input.json");
        const output = path.join(root, "output.json");
        fs.writeFileSync(input, JSON.stringify({ grants: [] }));
        const run = spawnSync(process.execPath, [path.join(workspace, "src", "cli.js"), "--input", input, "--now", "invalid", "--output", output], { encoding: "utf8", cwd: workspace });
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
