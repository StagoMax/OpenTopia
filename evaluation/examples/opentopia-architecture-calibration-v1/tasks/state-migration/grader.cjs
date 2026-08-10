const crypto = require("node:crypto"); const path = require("node:path");
const { runChecks, temporaryDirectory } = require("../../grader-kit.cjs");
const workspace = path.resolve(process.argv[2] || ""); const phase = process.argv[3] || "full"; const seed = path.join(__dirname, "seed");

runChecks({ workspace, seed, phase, protectedPaths: ["MIGRATION.md", "test", "legacy"], checks: [
  { id: "python-tests", phase: "plan", run: async ({ assert, spawnSync, workspace }) => {
    const run = spawnSync("python", ["-m", "unittest", "discover", "-s", "test", "-v"], { cwd: workspace, encoding: "utf8" }); assert.equal(run.status, 0, run.stderr || run.stdout);
  } },
  { id: "saved-plan-contract", phase: "plan", run: async ({ assert, crypto, fs, path, workspace }) => {
    assert.equal(fs.existsSync(path.join(workspace, "migrated")), false);
    const plan = JSON.parse(fs.readFileSync(path.join(workspace, "migration-plan.json"), "utf8"));
    const users = fs.readFileSync(path.join(workspace, "legacy/users.json")); const sessions = fs.readFileSync(path.join(workspace, "legacy/sessions.json"));
    assert.equal(plan.source_sha256, crypto.createHash("sha256").update(users).update(sessions).digest("hex"));
    assert.deepEqual(plan.accounts.map((item) => item.account_id), ["alice", "bob"]);
    assert.equal(plan.sessions[1].expires_at, "2026-09-02T00:30:00Z");
  } },
  { id: "migrated-final-state", phase: "full", run: async ({ assert, crypto, fs, path, workspace }) => {
    const root = path.join(workspace, "migrated"); const accounts = JSON.parse(fs.readFileSync(path.join(root, "accounts.json"), "utf8")); const sessions = JSON.parse(fs.readFileSync(path.join(root, "sessions.json"), "utf8")); const manifest = JSON.parse(fs.readFileSync(path.join(root, "manifest.json"), "utf8"));
    assert.deepEqual(accounts.map((item) => item.account_id), ["alice", "bob"]); assert.deepEqual(sessions.map((item) => item.account_id), ["alice", "bob"]); assert.deepEqual(manifest.counts, { accounts: 2, sessions: 2 });
    assert.equal(sessions[0].token_hash, crypto.createHash("sha256").update("secret-a").digest("hex"));
  } },
  { id: "collision-and-source-change-rejected", phase: "full", run: async ({ assert, fs, path, spawnSync, workspace }) => {
    const root = temporaryDirectory("migration-hidden-"); const legacy = path.join(root, "legacy"); fs.mkdirSync(legacy);
    fs.writeFileSync(path.join(legacy, "users.json"), JSON.stringify([{ id: "A", name: "A", email: "x@y.z" }, { id: "a", name: "B", email: "b@y.z" }])); fs.writeFileSync(path.join(legacy, "sessions.json"), "[]");
    const run = spawnSync("python", [path.join(workspace, "migration.py"), "--dry-run", "--source", legacy, "--plan", path.join(root, "plan.json")], { cwd: workspace, encoding: "utf8" }); assert.notEqual(run.status, 0);
  } }
] }).catch((error) => { console.error(error); process.exitCode = 2; });
