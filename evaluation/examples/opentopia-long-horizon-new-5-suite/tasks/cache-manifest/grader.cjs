const path = require("node:path");
const { runGrader } = require("../../grader-kit.cjs");

const workspace = path.resolve(process.argv[2] || "");
const phase = process.argv[3] || "full";
const seed = path.join(__dirname, "seed");
const aHash = "a".repeat(64);
const bHash = "b".repeat(64);
const cHash = "c".repeat(64);

runGrader({
  workspace,
  phase,
  seed,
  modulePath: "src/cache.js",
  exports: ["validateEntries", "diffCacheManifest", "summarizeDiff"],
  libraryChecks: [
    ["entry-validation", ({ assert, subject }) => {
      assert.ok(subject, "module did not load");
      const base = { path: "a/file", size: 1, sha256: aHash };
      assert.throws(() => subject.validateEntries([base, { ...base }]), /duplicate|unique/i);
      for (const entry of [
        { ...base, path: "/absolute" },
        { ...base, path: "../escape" },
        { ...base, path: "a//b" },
        { ...base, path: "C:/drive" },
        { ...base, size: -1 },
        { ...base, sha256: "abc" },
      ]) assert.throws(() => subject.validateEntries([entry]));
    }],
    ["manifest-diff-contract", ({ assert, subject }) => {
      const expected = [
        { path: "missing", size: 1, sha256: aHash },
        { path: "both", size: 2, sha256: bHash },
        { path: "hash", size: 3, sha256: cHash },
        { path: "same", size: 4, sha256: aHash },
      ];
      const observed = [
        { path: "new", size: 1, sha256: aHash },
        { path: "both", size: 9, sha256: cHash },
        { path: "hash", size: 3, sha256: bHash },
        { path: "same", size: 4, sha256: aHash },
      ];
      const before = JSON.stringify({ expected, observed });
      const diff = subject.diffCacheManifest(expected, observed);
      assert.equal(JSON.stringify({ expected, observed }), before, "inputs were mutated");
      assert.deepEqual(diff.missing.map((entry) => entry.path), ["missing"]);
      assert.deepEqual(diff.unexpected.map((entry) => entry.path), ["new"]);
      assert.deepEqual(diff.changed.map((entry) => [entry.path, entry.reasons]), [["both", ["size", "sha256"]], ["hash", ["sha256"]]]);
      assert.deepEqual(diff.unchanged.map((entry) => entry.path), ["same"]);
    }],
    ["manifest-summary", ({ assert, subject }) => {
      const diff = subject.diffCacheManifest([{ path: "a", size: 1, sha256: aHash }], [{ path: "a", size: 1, sha256: aHash }]);
      assert.deepEqual(subject.summarizeDiff(diff), {
        expected: 1, observed: 1, missing: 0, unexpected: 0, changed: 0, unchanged: 1, valid: true,
      });
    }],
  ],
  fullChecks: [
    ["cli-valid-contract", ({ assert, fs, os, path, spawnSync, workspace }) => {
      const root = fs.mkdtempSync(path.join(os.tmpdir(), "cache-valid-"));
      try {
        const expected = path.join(root, "expected.json");
        const observed = path.join(root, "observed.json");
        const output = path.join(root, "output.json");
        const entries = [{ path: "a", size: 1, sha256: aHash }];
        fs.writeFileSync(expected, JSON.stringify(entries));
        fs.writeFileSync(observed, JSON.stringify(entries));
        const run = spawnSync(process.execPath, [path.join(workspace, "src", "cli.js"), "--expected", expected, "--observed", observed, "--output", output], { encoding: "utf8", cwd: workspace });
        assert.equal(run.status, 0, run.stderr);
        assert.equal(run.stdout, "Verified 1 expected entries: 0 missing, 0 unexpected, 0 changed.\n");
        assert.ok(fs.readFileSync(output, "utf8").endsWith("\n"));
      } finally { fs.rmSync(root, { recursive: true, force: true }); }
    }],
    ["cli-difference-contract", ({ assert, fs, os, path, spawnSync, workspace }) => {
      const root = fs.mkdtempSync(path.join(os.tmpdir(), "cache-diff-"));
      try {
        const expected = path.join(root, "expected.json");
        const observed = path.join(root, "observed.json");
        const output = path.join(root, "output.json");
        fs.writeFileSync(expected, JSON.stringify([{ path: "a", size: 1, sha256: aHash }]));
        fs.writeFileSync(observed, JSON.stringify([]));
        const run = spawnSync(process.execPath, [path.join(workspace, "src", "cli.js"), "--expected", expected, "--observed", observed, "--output", output], { encoding: "utf8", cwd: workspace });
        assert.equal(run.status, 1, run.stderr);
        assert.equal(run.stdout, "Verified 1 expected entries: 1 missing, 0 unexpected, 0 changed.\n");
        assert.equal(JSON.parse(fs.readFileSync(output, "utf8")).summary.valid, false);
      } finally { fs.rmSync(root, { recursive: true, force: true }); }
    }],
    ["cli-invalid-contract", ({ assert, fs, os, path, spawnSync, workspace }) => {
      const root = fs.mkdtempSync(path.join(os.tmpdir(), "cache-invalid-"));
      try {
        const expected = path.join(root, "expected.json");
        const observed = path.join(root, "observed.json");
        const output = path.join(root, "output.json");
        fs.writeFileSync(expected, JSON.stringify([{ path: "../bad", size: 1, sha256: aHash }]));
        fs.writeFileSync(observed, JSON.stringify([]));
        const run = spawnSync(process.execPath, [path.join(workspace, "src", "cli.js"), "--expected", expected, "--observed", observed, "--output", output], { encoding: "utf8", cwd: workspace });
        assert.equal(run.status, 2);
        assert.ok(run.stderr.trim());
        assert.equal(fs.existsSync(output), false);
      } finally { fs.rmSync(root, { recursive: true, force: true }); }
    }],
  ],
}).catch((error) => {
  process.stderr.write(`${error.stack || error}\n`);
  process.exitCode = 2;
});
