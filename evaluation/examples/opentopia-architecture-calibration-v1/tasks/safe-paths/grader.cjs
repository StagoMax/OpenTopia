const path = require("node:path");
const { runChecks } = require("../../grader-kit.cjs");
const workspace = path.resolve(process.argv[2] || "");
const seed = path.join(__dirname, "seed");

runChecks({ workspace, seed, checks: [
  { id: "safe-plan-and-input-isolation", run: async ({ assert, importFresh, path, workspace }) => {
    const { buildExtractionPlan } = await importFresh(path.join(workspace, "src/extract.js"));
    const root = path.resolve(workspace, "out");
    const entries = [{ path: "z.txt", type: "file" }, { path: "a/link", type: "symlink", target: "../target.txt" }, { path: "a/target.txt", type: "file" }];
    const before = JSON.stringify(entries);
    const plan = buildExtractionPlan(root, entries);
    assert.equal(JSON.stringify(entries), before);
    assert.deepEqual(plan.map((entry) => [entry.path, entry.type, entry.target]), [["a/link", "symlink", "../target.txt"], ["a/target.txt", "file", null], ["z.txt", "file", null]]);
    for (const entry of plan) assert.ok(entry.destination.startsWith(root + path.sep));
  } },
  { id: "rejects-cross-platform-escapes", run: async ({ assert, importFresh, path, workspace }) => {
    const { buildExtractionPlan } = await importFresh(path.join(workspace, "src/extract.js"));
    for (const candidate of ["../x", "a/../../x", "/etc/passwd", "C:\\temp\\x", "\\\\server\\share\\x", "a//b", "./a", "a/./b"]) {
      assert.throws(() => buildExtractionPlan("out", [{ path: candidate, type: "file" }]), undefined, candidate);
    }
    assert.throws(() => buildExtractionPlan("out", [{ path: "a", type: "file" }, { path: "a", type: "directory" }]), /duplicate/i);
    assert.throws(() => buildExtractionPlan("out", [{ path: "a/link", type: "symlink", target: "../../escape" }]), /target|escape|traversal/i);
  } },
  { id: "public-tests", run: async ({ assert, spawnSync, workspace }) => {
    const run = spawnSync("npm", ["test", "--silent"], { cwd: workspace, encoding: "utf8", shell: process.platform === "win32" });
    assert.equal(run.status, 0, run.stderr || run.stdout);
  } }
] }).catch((error) => { console.error(error); process.exitCode = 2; });
