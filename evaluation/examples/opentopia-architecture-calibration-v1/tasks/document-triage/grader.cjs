const path = require("node:path");
const { runChecks, temporaryDirectory } = require("../../grader-kit.cjs");
const workspace = path.resolve(process.argv[2] || "");
const seed = path.join(__dirname, "seed");

runChecks({ workspace, seed, protectedPaths: ["OPERATIONS.md", "test"], checks: [
  { id: "classification-and-extraction", run: async ({ assert, importFresh, path, workspace }) => {
    const subject = await importFresh(path.join(workspace, "src/documents.js"));
    const invoice = "invoice number: Z 9\nTax: $0.07\nTotal: 1.10\nAmount Due: 0.20";
    assert.equal(subject.classifyDocument(invoice), "invoice");
    assert.deepEqual(subject.extractInvoice(invoice), { invoiceNumber: "Z 9", totalCents: 110, taxCents: 7 });
    assert.equal(subject.classifyDocument("Invoice Number: missing-total"), "other");
    assert.throws(() => subject.extractInvoice("Invoice Number: X\nTotal: 1.234"), /amount|decimal|total/i);
  } },
  { id: "cli-atomic-move-and-summary", run: async ({ assert, fs, path, spawnSync, workspace }) => {
    const root = temporaryDirectory("docs-hidden-"); const input = path.join(root, "in"); const output = path.join(root, "out"); fs.mkdirSync(input);
    fs.writeFileSync(path.join(input, "z.txt"), "Invoice Number: Z\nGST 0.50\nAmount Due 4.00");
    fs.writeFileSync(path.join(input, "a.txt"), "Invoice Number: A\nTotal 2.25");
    fs.writeFileSync(path.join(input, "memo.txt"), "Total tasks: 3");
    const run = spawnSync(process.execPath, [path.join(workspace, "src/cli.js"), "--input", input, "--output", output], { encoding: "utf8", cwd: workspace });
    assert.equal(run.status, 0, run.stderr);
    assert.deepEqual(fs.readdirSync(input), []);
    assert.deepEqual(fs.readdirSync(path.join(output, "invoices")).sort(), ["a.txt", "summary.csv", "z.txt"]);
    assert.equal(fs.readFileSync(path.join(output, "invoices/summary.csv"), "utf8"), "filename,invoice_number,total_cents,tax_cents\na.txt,A,225,0\nz.txt,Z,400,50\nTOTAL,,625,50\n");
    assert.deepEqual(fs.readdirSync(path.join(output, "other")), ["memo.txt"]);
  } },
  { id: "seeded-final-state", run: async ({ assert, fs, path, workspace }) => {
    assert.deepEqual(fs.readdirSync(path.join(workspace, "documents")), []);
    const csv = fs.readFileSync(path.join(workspace, "processed/invoices/summary.csv"), "utf8");
    assert.equal(csv, "filename,invoice_number,total_cents,tax_cents\ninvoice-a.txt,A-100,1200,200\ninvoice-b.txt,B-2,525,0\nTOTAL,,1725,200\n");
    assert.ok(fs.existsSync(path.join(workspace, "processed/other/notes.txt")));
  } },
  { id: "public-tests", run: async ({ assert, spawnSync, workspace }) => {
    const run = spawnSync("npm", ["test", "--silent"], { cwd: workspace, encoding: "utf8", shell: process.platform === "win32" });
    assert.equal(run.status, 0, run.stderr || run.stdout);
  } }
] }).catch((error) => { console.error(error); process.exitCode = 2; });
