const path = require("node:path");
const { runChecks } = require("../../grader-kit.cjs");
const workspace = path.resolve(process.argv[2] || "");
const seed = path.join(__dirname, "seed");

runChecks({ workspace, seed, checks: [
  { id: "waves-cache-and-transitive-invalidation", run: async ({ assert, importFresh, path, workspace }) => {
    const { planBuild } = await importFresh(path.join(workspace, "src/planner.js"));
    const packages = [
      { name: "web", dependencies: ["ui", "core"], inputs: { z: "1", a: "2" } },
      { name: "core", dependencies: [], inputs: { code: "v2" } },
      { name: "ui", dependencies: ["core"], inputs: { code: "same" } },
      { name: "docs", dependencies: [], inputs: { code: "same" } }
    ];
    const initial = planBuild(packages.map((item) => ({ ...item, inputs: item.name === "core" ? { code: "v1" } : item.inputs })), {});
    const previous = { ...initial.cache, core: initial.cache.core };
    const before = JSON.stringify(packages);
    const result = planBuild(packages, previous);
    assert.equal(JSON.stringify(packages), before);
    assert.deepEqual(result.waves, [["core", "docs"], ["ui"], ["web"]]);
    assert.deepEqual(result.rebuild, ["core", "ui", "web"]);
    assert.deepEqual(Object.keys(result.cache), ["core", "docs", "ui", "web"]);
  } },
  { id: "deterministic-hash-and-graph-errors", run: async ({ assert, importFresh, path, workspace }) => {
    const { hashInputs } = await importFresh(path.join(workspace, "src/hash.js"));
    const { planBuild } = await importFresh(path.join(workspace, "src/planner.js"));
    assert.equal(hashInputs({ b: 2, a: 1 }), hashInputs({ a: 1, b: 2 }));
    assert.throws(() => planBuild([{ name: "a", dependencies: ["missing"], inputs: {} }]));
    assert.throws(() => planBuild([{ name: "a", dependencies: ["b"], inputs: {} }, { name: "b", dependencies: ["a"], inputs: {} }]), /cycle/i);
  } },
  { id: "public-tests", run: async ({ assert, spawnSync, workspace }) => {
    const run = spawnSync("npm", ["test", "--silent"], { cwd: workspace, encoding: "utf8", shell: process.platform === "win32" });
    assert.equal(run.status, 0, run.stderr || run.stdout);
  } }
] }).catch((error) => { console.error(error); process.exitCode = 2; });
