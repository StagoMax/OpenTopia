const path = require("node:path");
const { runChecks } = require("../../grader-kit.cjs");
const workspace = path.resolve(process.argv[2] || "");
const seed = path.join(__dirname, "seed");

runChecks({ workspace, seed, checks: [
  { id: "ordering-and-concurrency", run: async ({ assert, importFresh, path, workspace }) => {
    const { runPool } = await importFresh(path.join(workspace, "src/pool.js"));
    let active = 0; let peak = 0;
    const output = await runPool([25, 5, 15, 1], async (delay, index) => {
      active += 1; peak = Math.max(peak, active);
      await new Promise((resolve) => setTimeout(resolve, delay));
      active -= 1; return `r${index}`;
    }, { concurrency: 2 });
    assert.deepEqual(output, ["r0", "r1", "r2", "r3"]);
    assert.ok(peak <= 2);
  } },
  { id: "abort-stops-queued-work", run: async ({ assert, importFresh, path, workspace }) => {
    const { runPool } = await importFresh(path.join(workspace, "src/pool.js"));
    const controller = new AbortController();
    const started = [];
    const promise = runPool([0, 1, 2, 3], async (value) => {
      started.push(value);
      if (value === 0) controller.abort();
      await new Promise((resolve) => setTimeout(resolve, 10));
      return value;
    }, { concurrency: 1, signal: controller.signal });
    await assert.rejects(promise, (error) => error?.name === "AbortError");
    assert.deepEqual(started, [0]);
    const pre = new AbortController(); pre.abort(); let calls = 0;
    await assert.rejects(runPool([1], () => { calls += 1; }, { signal: pre.signal }), (error) => error?.name === "AbortError");
    assert.equal(calls, 0);
  } },
  { id: "sync-throw-rejects-and-stops", run: async ({ assert, importFresh, path, workspace }) => {
    const { runPool } = await importFresh(path.join(workspace, "src/pool.js"));
    const started = [];
    await assert.rejects(runPool([0, 1, 2], (value) => { started.push(value); if (value === 0) throw new Error("boom"); return value; }, { concurrency: 1 }), /boom/);
    assert.deepEqual(started, [0]);
  } },
  { id: "public-tests", run: async ({ assert, spawnSync, workspace }) => {
    const run = spawnSync("npm", ["test", "--silent"], { cwd: workspace, encoding: "utf8", shell: process.platform === "win32" });
    assert.equal(run.status, 0, run.stderr || run.stdout);
  } }
] }).catch((error) => { console.error(error); process.exitCode = 2; });
