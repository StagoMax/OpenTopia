const path = require("node:path");
const { Readable } = require("node:stream");
const zlib = require("node:zlib");
const { runChecks, temporaryDirectory } = require("../../grader-kit.cjs");
const workspace = path.resolve(process.argv[2] || "");
const seed = path.join(__dirname, "seed");

runChecks({ workspace, seed, checks: [
  { id: "filter-validation-and-p95", run: async ({ assert, importFresh, path, workspace }) => {
    const { summarizeLog } = await importFresh(path.join(workspace, "src/logs.js"));
    const rows = [5, 1, 9, 3].map((durationMs, index) => ({ timestamp: `2026-01-01T00:0${index}:00Z`, service: index % 2 ? "worker" : "api", level: index === 2 ? "error" : "info", requestId: `r${index}`, durationMs }));
    const result = await summarizeLog(Readable.from(rows.map((row) => `${JSON.stringify(row)}\n`)), { since: "2026-01-01T00:01:00Z", until: "2026-01-01T00:03:00Z" });
    assert.equal(result.events, 3); assert.equal(result.firstTimestamp, "2026-01-01T00:01:00.000Z"); assert.equal(result.lastTimestamp, "2026-01-01T00:03:00.000Z");
    assert.deepEqual(result.services, [{ service: "api", events: 1, errors: 1, p95DurationMs: 9 }, { service: "worker", events: 2, errors: 0, p95DurationMs: 3 }]);
    await assert.rejects(summarizeLog(Readable.from([`${JSON.stringify(rows[0])}\n${JSON.stringify(rows[0])}\n`])), /duplicate|request/i);
  } },
  { id: "gzip-cli", run: async ({ assert, fs, path, spawnSync, workspace }) => {
    const root = temporaryDirectory("log-hidden-"); const input = path.join(root, "events.jsonl.gz"); const output = path.join(root, "summary.json");
    const rows = Array.from({ length: 2000 }, (_, index) => JSON.stringify({ timestamp: new Date(Date.UTC(2026, 0, 1, 0, 0, index)).toISOString(), service: index % 2 ? "b" : "a", level: index % 10 === 0 ? "error" : "info", requestId: `id-${index}`, durationMs: index % 101 })).join("\n") + "\n";
    fs.writeFileSync(input, zlib.gzipSync(rows));
    const run = spawnSync(process.execPath, [path.join(workspace, "src/cli.js"), "--input", input, "--output", output], { cwd: workspace, encoding: "utf8", maxBuffer: 1024 * 1024 });
    assert.equal(run.status, 0, run.stderr); const summary = JSON.parse(fs.readFileSync(output, "utf8")); assert.equal(summary.events, 2000); assert.ok(fs.readFileSync(output, "utf8").endsWith("\n"));
    const source = fs.readFileSync(path.join(workspace, "src/cli.js"), "utf8"); assert.doesNotMatch(source, /readFileSync|readFile\s*\(/);
  } },
  { id: "public-tests", run: async ({ assert, spawnSync, workspace }) => {
    const run = spawnSync("npm", ["test", "--silent"], { cwd: workspace, encoding: "utf8", shell: process.platform === "win32" });
    assert.equal(run.status, 0, run.stderr || run.stdout);
  } }
] }).catch((error) => { console.error(error); process.exitCode = 2; });
