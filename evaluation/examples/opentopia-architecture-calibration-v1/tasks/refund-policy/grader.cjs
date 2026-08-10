const path = require("node:path");
const { runChecks, temporaryDirectory } = require("../../grader-kit.cjs");
const workspace = path.resolve(process.argv[2] || ""); const phase = process.argv[3] || "full"; const seed = path.join(__dirname, "seed");

runChecks({ workspace, seed, phase, protectedPaths: ["POLICY.md", "test", "orders.json", "requests.json"], checks: [
  { id: "policy-boundaries", phase: "library", run: async ({ assert, importFresh, path, workspace }) => {
    const { decideRefund } = await importFresh(path.join(workspace, "src/policy.js"));
    const physical = { id: "p", kind: "physical", paidCents: 1000, refundedCents: 200, deliveredAt: "2026-01-01T00:00:00Z", fraudHold: false };
    assert.equal(decideRefund(physical, { id: "a", orderId: "p", amountCents: 800, reason: "changed_mind" }, "2026-01-31T00:00:00Z").status, "approved");
    assert.equal(decideRefund(physical, { id: "b", orderId: "p", amountCents: 801, reason: "changed_mind" }, "2026-01-31T00:00:00Z").status, "rejected");
    assert.equal(decideRefund(physical, { id: "c", orderId: "p", amountCents: 100, reason: "damaged" }, "2026-02-15T00:00:00Z").status, "rejected");
    assert.equal(decideRefund(physical, { id: "d", orderId: "p", amountCents: 100, reason: "damaged", evidenceId: "e" }, "2026-03-31T00:00:00Z").status, "approved");
    const digital = { id: "d", kind: "digital", paidCents: 500, refundedCents: 0, purchasedAt: "2026-01-01T00:00:00Z", downloadedAt: "2026-01-02T00:00:00Z", fraudHold: false };
    assert.equal(decideRefund(digital, { id: "e", orderId: "d", amountCents: 100, reason: "changed_mind" }, "2026-01-03T00:00:00Z").status, "rejected");
  } },
  { id: "public-tests", phase: "library", run: async ({ assert, spawnSync, workspace }) => {
    const run = spawnSync("npm", ["test", "--silent"], { cwd: workspace, encoding: "utf8", shell: process.platform === "win32" }); assert.equal(run.status, 0, run.stderr || run.stdout);
  } },
  { id: "seeded-ledger-final-state", phase: "full", run: async ({ assert, fs, path, spawnSync, workspace }) => {
    const ledgerPath = path.join(workspace, "ledger.json"); const ledger = JSON.parse(fs.readFileSync(ledgerPath, "utf8"));
    assert.deepEqual(ledger, [{ transactionId: "refund:r-approved", requestId: "r-approved", orderId: "physical", amountCents: 1200 }]);
    const decisions = JSON.parse(fs.readFileSync(path.join(workspace, "decisions.json"), "utf8"));
    assert.deepEqual(decisions.map((item) => [item.requestId, item.status]), [["r-approved", "approved"], ["r-digital", "rejected"], ["r-held", "manual_review"]]);
    const rerun = spawnSync(process.execPath, [path.join(workspace, "src/cli.js"), "--orders", path.join(workspace, "orders.json"), "--requests", path.join(workspace, "requests.json"), "--ledger", ledgerPath, "--now", "2026-08-01T00:00:00Z"], { cwd: workspace, encoding: "utf8" });
    assert.equal(rerun.status, 0, rerun.stderr); assert.equal(JSON.parse(fs.readFileSync(ledgerPath, "utf8")).length, 1);
  } }
] }).catch((error) => { console.error(error); process.exitCode = 2; });
