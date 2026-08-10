const crypto = require("node:crypto");
const path = require("node:path");
const { runChecks } = require("../../grader-kit.cjs");
const workspace = path.resolve(process.argv[2] || "");
const phase = process.argv[3] || "full";
const seed = path.join(__dirname, "seed");
const frame = (sequence, operation) => {
  const checksum = crypto.createHash("sha256").update(JSON.stringify({ sequence, operation })).digest("hex");
  return JSON.stringify({ sequence, operation, checksum });
};

runChecks({ workspace, seed, phase, protectedPaths: ["INCIDENT.md", "test", "data"], checks: [
  { id: "library-replay-and-isolation", phase: "library", run: async ({ assert, importFresh, path, workspace }) => {
    const subject = await importFresh(path.join(workspace, "src/journal.js"));
    const text = [frame(8, { type: "put", key: "z", value: { n: 1 } }), frame(9, { type: "delete", key: "old" }), frame(10, { type: "put", key: "a", value: true }), '{"sequence":11'].join("\n");
    const frames = subject.parseJournal(text);
    const snapshot = { sequence: 7, records: { old: 3, middle: 2 } };
    const before = JSON.stringify({ snapshot, frames });
    assert.deepEqual(subject.recover(snapshot, frames), { sequence: 10, records: { a: true, middle: 2, z: { n: 1 } } });
    assert.equal(JSON.stringify({ snapshot, frames }), before);
  } },
  { id: "library-corruption-detection", phase: "library", run: async ({ assert, importFresh, path, workspace }) => {
    const subject = await importFresh(path.join(workspace, "src/journal.js"));
    assert.throws(() => subject.parseJournal(`${frame(1, { type: "put", key: "x", value: 1 })}\nnot-json\n${frame(2, { type: "delete", key: "x" })}`));
    const bad = JSON.parse(frame(1, { type: "delete", key: "x" })); bad.checksum = "0".repeat(64);
    assert.throws(() => subject.parseJournal(JSON.stringify(bad)), /checksum/i);
    assert.throws(() => subject.recover({ sequence: 1, records: {} }, JSON.parse(`[${frame(3, { type: "delete", key: "x" })}]`)), /sequence|consecutive/i);
  } },
  { id: "public-tests", phase: "library", run: async ({ assert, spawnSync, workspace }) => {
    const run = spawnSync("npm", ["test", "--silent"], { cwd: workspace, encoding: "utf8", shell: process.platform === "win32" });
    assert.equal(run.status, 0, run.stderr || run.stdout);
  } },
  { id: "cli-recovers-seeded-incident", phase: "full", run: async ({ assert, fs, path, spawnSync, workspace }) => {
    const output = path.join(workspace, "recovered.json");
    const run = spawnSync(process.execPath, [path.join(workspace, "src/cli.js"), "--snapshot", path.join(workspace, "data/snapshot.json"), "--journal", path.join(workspace, "data/journal.log"), "--output", output], { cwd: workspace, encoding: "utf8" });
    assert.equal(run.status, 0, run.stderr);
    assert.equal(run.stdout, "Recovered sequence 5 with 2 records.\n");
    assert.deepEqual(JSON.parse(fs.readFileSync(output, "utf8")), { sequence: 5, records: { alpha: 7, beta: 2 } });
    assert.ok(fs.readFileSync(output, "utf8").endsWith("\n"));
  } }
] }).catch((error) => { console.error(error); process.exitCode = 2; });
