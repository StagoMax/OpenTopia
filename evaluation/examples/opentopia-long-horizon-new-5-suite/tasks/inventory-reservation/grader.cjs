const path = require("node:path");
const { runGrader } = require("../../grader-kit.cjs");

const workspace = path.resolve(process.argv[2] || "");
const phase = process.argv[3] || "full";
const seed = path.join(__dirname, "seed");

runGrader({
  workspace,
  phase,
  seed,
  modulePath: "src/inventory.js",
  exports: ["normalizeInventory", "planReservations", "summarizeReservations"],
  libraryChecks: [
    ["inventory-validation", ({ assert, subject }) => {
      assert.ok(subject, "module did not load");
      assert.throws(() => subject.normalizeInventory([{ sku: "A", onHand: 1 }, { sku: "A", onHand: 2 }]), /duplicate|unique/i);
      for (const record of [
        { sku: "", onHand: 1 },
        { sku: "A", onHand: -1 },
        { sku: "A", onHand: 1.5 },
        { sku: "A", onHand: 1, reserved: 2 },
      ]) assert.throws(() => subject.normalizeInventory([record]));
    }],
    ["reservation-priority-and-isolation", ({ assert, subject }) => {
      const inventory = [{ sku: "X", onHand: 7, reserved: 2 }, { sku: "Y", onHand: 1 }];
      const orders = [
        { id: "z", sku: "X", quantity: 4, priority: 5 },
        { id: "a", sku: "X", quantity: 3, priority: 5 },
        { id: "back", sku: "Y", quantity: 2, priority: 9 },
      ];
      const before = JSON.stringify({ inventory, orders });
      const plan = subject.planReservations(inventory, orders);
      assert.equal(JSON.stringify({ inventory, orders }), before, "inputs were mutated");
      assert.deepEqual(plan.allocations, [
        { id: "back", sku: "Y", requested: 2, allocated: 1, status: "partial" },
        { id: "a", sku: "X", requested: 3, allocated: 3, status: "filled" },
        { id: "z", sku: "X", requested: 4, allocated: 2, status: "partial" },
      ]);
      assert.deepEqual(plan.inventory, [
        { sku: "X", onHand: 7, reserved: 7, available: 0 },
        { sku: "Y", onHand: 1, reserved: 1, available: 0 },
      ]);
    }],
    ["order-validation-and-summary", ({ assert, subject }) => {
      const stock = [{ sku: "A", onHand: 0 }];
      assert.throws(() => subject.planReservations(stock, [{ id: "x", sku: "B", quantity: 1, priority: 0 }]), /sku|unknown/i);
      assert.throws(() => subject.planReservations(stock, [{ id: "x", sku: "A", quantity: 0, priority: 0 }]));
      const plan = subject.planReservations(stock, [{ id: "x", sku: "A", quantity: 2, priority: 0 }]);
      assert.deepEqual(subject.summarizeReservations(plan), {
        orders: 1, filled: 0, partial: 0, backordered: 1,
        requestedUnits: 2, allocatedUnits: 0, remainingUnits: 0,
      });
    }],
  ],
  fullChecks: [
    ["cli-success-contract", ({ assert, fs, os, path, spawnSync, workspace }) => {
      const root = fs.mkdtempSync(path.join(os.tmpdir(), "inventory-eval-"));
      try {
        const input = path.join(root, "input.json");
        const output = path.join(root, "output.json");
        fs.writeFileSync(input, JSON.stringify({ inventory: [{ sku: "A", onHand: 4 }], orders: [{ id: "one", sku: "A", quantity: 3, priority: 1 }] }));
        const run = spawnSync(process.execPath, [path.join(workspace, "src", "cli.js"), "--input", input, "--output", output], { encoding: "utf8", cwd: workspace });
        assert.equal(run.status, 0, run.stderr);
        assert.equal(run.stdout, "Planned 1 orders: 3 units allocated, 1 units remaining.\n");
        const raw = fs.readFileSync(output, "utf8");
        assert.ok(raw.endsWith("\n"));
        assert.equal(JSON.parse(raw).summary.allocatedUnits, 3);
      } finally { fs.rmSync(root, { recursive: true, force: true }); }
    }],
    ["cli-failure-contract", ({ assert, fs, os, path, spawnSync, workspace }) => {
      const root = fs.mkdtempSync(path.join(os.tmpdir(), "inventory-invalid-"));
      try {
        const input = path.join(root, "input.json");
        const output = path.join(root, "output.json");
        fs.writeFileSync(input, JSON.stringify({ inventory: [], orders: [{ id: "x", sku: "missing", quantity: 1, priority: 0 }] }));
        const run = spawnSync(process.execPath, [path.join(workspace, "src", "cli.js"), "--input", input, "--output", output], { encoding: "utf8", cwd: workspace });
        assert.notEqual(run.status, 0);
        assert.ok(run.stderr.trim());
        assert.equal(fs.existsSync(output), false);
      } finally { fs.rmSync(root, { recursive: true, force: true }); }
    }],
  ],
}).catch((error) => {
  process.stderr.write(`${error.stack || error}\n`);
  process.exitCode = 2;
});
