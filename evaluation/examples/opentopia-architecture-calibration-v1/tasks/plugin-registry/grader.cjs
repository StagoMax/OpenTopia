const path = require("node:path"); const { runChecks } = require("../../grader-kit.cjs");
const workspace = path.resolve(process.argv[2] || ""); const phase = process.argv[3] || "full"; const seed = path.join(__dirname, "seed");

runChecks({ workspace, seed, phase, checks: [
  { id: "diagnosis-evidence", phase: "diagnose", run: async ({ assert, fs, path, workspace }) => {
    const diagnosis = JSON.parse(fs.readFileSync(path.join(workspace, "diagnosis.json"), "utf8"));
    assert.equal(diagnosis.rootCauseFile.replaceAll("\\", "/"), "src/merge.js"); assert.equal(diagnosis.rootCauseSymbol, "selectDefinitions"); assert.match(diagnosis.reproductionCommand, /npm\s+test/i); assert.match(diagnosis.observedFailure, /disabled|fallback|shadow/i);
    assert.equal(fs.readFileSync(path.join(workspace, "src/merge.js"), "utf8"), fs.readFileSync(path.join(seed, "src/merge.js"), "utf8"));
  } },
  { id: "precedence-fallback-and-order", phase: "full", run: async ({ assert, importFresh, path, workspace }) => {
    const { resolveRegistry } = await importFresh(path.join(workspace, "src/registry.js"));
    const layers = [
      { scope: "workspace", plugins: [{ id: "fmt", version: "3", enabled: false, dependencies: [] }, { id: "app", version: "1", enabled: true, dependencies: ["core", "fmt"] }] },
      { scope: "system", plugins: [{ id: "fmt", version: "1", enabled: true, dependencies: [] }, { id: "core", version: "1", enabled: true, dependencies: [] }] },
      { scope: "user", plugins: [{ id: "fmt", version: "2", enabled: true, dependencies: [] }] }
    ];
    const before = JSON.stringify(layers); const result = resolveRegistry(layers); assert.equal(JSON.stringify(layers), before);
    assert.deepEqual(result.map((item) => `${item.id}@${item.version}`), ["core@1", "fmt@2", "app@1"]);
  } },
  { id: "invalid-graphs", phase: "full", run: async ({ assert, importFresh, path, workspace }) => {
    const { resolveRegistry } = await importFresh(path.join(workspace, "src/registry.js"));
    assert.throws(() => resolveRegistry([{ scope: "system", plugins: [{ id: "a", enabled: true, dependencies: ["missing"] }] }]));
    assert.throws(() => resolveRegistry([{ scope: "system", plugins: [{ id: "a", enabled: true, dependencies: ["b"] }, { id: "b", enabled: true, dependencies: ["a"] }] }]), /cycle/i);
    assert.throws(() => resolveRegistry([{ scope: "system", plugins: [{ id: "a", enabled: true, dependencies: [] }, { id: "a", enabled: true, dependencies: [] }] }]), /duplicate/i);
  } },
  { id: "public-tests", phase: "full", run: async ({ assert, spawnSync, workspace }) => { const run = spawnSync("npm", ["test", "--silent"], { cwd: workspace, encoding: "utf8", shell: process.platform === "win32" }); assert.equal(run.status, 0, run.stderr || run.stdout); } }
] }).catch((error) => { console.error(error); process.exitCode = 2; });
