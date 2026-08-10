const path = require("node:path"); const { runChecks, temporaryDirectory } = require("../../grader-kit.cjs");
const workspace = path.resolve(process.argv[2] || ""); const phase = process.argv[3] || "full"; const seed = path.join(__dirname, "seed");

runChecks({ workspace, seed, phase, protectedPaths: ["SPEC.md", "test", "scenario.json"], checks: [
  { id: "enqueue-lease-persistence", phase: "core", run: async ({ assert, importFresh, path, workspace }) => {
    const { DurableQueue } = await importFresh(path.join(workspace, "src/queue.js")); const root = temporaryDirectory("queue-core-"); const state = path.join(root, "state.json"); let now = new Date("2026-01-01T00:00:00Z");
    let queue = new DurableQueue(state, () => now); queue.enqueue({ id: "b", payload: 2, availableAt: now }); queue.enqueue({ id: "a", payload: 1, availableAt: now }); assert.equal(queue.lease("w", 1000).id, "a");
    queue = new DurableQueue(state, () => now); assert.equal(queue.snapshot().find((job) => job.id === "a").owner, "w"); assert.throws(() => queue.enqueue({ id: "a", payload: 3, availableAt: now }), /duplicate/i);
  } },
  { id: "public-tests-core", phase: "core", run: async ({ assert, spawnSync, workspace }) => { const run = spawnSync("npm", ["test", "--silent"], { cwd: workspace, encoding: "utf8", shell: process.platform === "win32" }); assert.equal(run.status, 0, run.stderr || run.stdout); } },
  { id: "expiry-ack-retry-and-isolation", phase: "recovery", run: async ({ assert, importFresh, path, workspace }) => {
    const { DurableQueue } = await importFresh(path.join(workspace, "src/queue.js")); const root = temporaryDirectory("queue-recovery-"); const state = path.join(root, "state.json"); let now = new Date("2026-01-01T00:00:00Z");
    const queue = new DurableQueue(state, () => now); queue.enqueue({ id: "x", payload: { n: 1 }, availableAt: now }); assert.equal(queue.lease("w1", 1000).attempts, 1); assert.throws(() => queue.ack("w2", "x"));
    now = new Date("2026-01-01T00:00:02Z"); assert.equal(queue.lease("w2", 1000).attempts, 2); queue.retry("w2", "x", 3000); assert.equal(queue.lease("w3", 1000), null); now = new Date("2026-01-01T00:00:05Z"); assert.equal(queue.lease("w3", 1000).id, "x"); queue.ack("w3", "x"); assert.equal(queue.snapshot()[0].status, "completed");
  } },
  { id: "scenario-final-state", phase: "full", run: async ({ assert, fs, path, workspace }) => {
    const state = JSON.parse(fs.readFileSync(path.join(workspace, "queue-state.json"), "utf8")); const result = JSON.parse(fs.readFileSync(path.join(workspace, "scenario-result.json"), "utf8"));
    const jobs = state.jobs.sort((a, b) => a.id.localeCompare(b.id)); assert.deepEqual(jobs.map((job) => [job.id, job.status, job.attempts]), [["a", "completed", 2], ["b", "pending", 1]]); assert.equal(jobs[1].availableAt, "2026-08-01T00:00:10.000Z"); assert.equal(result.length, 7);
  } }
] }).catch((error) => { console.error(error); process.exitCode = 2; });
