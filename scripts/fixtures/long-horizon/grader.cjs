const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { pathToFileURL } = require("node:url");
const { spawnSync } = require("node:child_process");

const workspace = path.resolve(process.argv[2] || "");
const phase = process.argv[3] || "full";
const seed = path.join(__dirname, "seed");
if (!workspace || !["library", "full"].includes(phase)) {
  process.stderr.write("usage: grader.cjs <workspace> <library|full>\n");
  process.exit(2);
}

const checks = [];

async function check(id, action) {
  try {
    await action();
    checks.push({ id, passed: true });
  } catch (error) {
    checks.push({
      id,
      passed: false,
      detail: String(error?.message || error).slice(0, 300),
    });
  }
}

function hash(filePath) {
  return crypto.createHash("sha256").update(fs.readFileSync(filePath)).digest("hex");
}

async function main() {
  let ledger;
  await check("module-loads", async () => {
    ledger = await import(
      `${pathToFileURL(path.join(workspace, "src", "ledger.js")).href}?eval=${Date.now()}`
    );
    for (const name of ["parseTransactions", "reconcileAccounts", "renderReport"]) {
      assert.equal(typeof ledger[name], "function", `${name} must be exported`);
    }
  });

  await check("csv-rfc4180-edge-cases", () => {
    assert.ok(ledger, "module did not load");
    const rows = ledger.parseTransactions(
      '\ufeffid,account,type,amount\r\n1,"North, Inc.",debit,10.2\r\n2,"Quote ""Desk""",credit,3\r\n\r\n',
    );
    assert.deepEqual(rows, [
      {
        id: "1",
        account: "North, Inc.",
        type: "debit",
        amountCents: 1020,
      },
      {
        id: "2",
        account: 'Quote "Desk"',
        type: "credit",
        amountCents: 300,
      },
    ]);
  });

  await check("csv-validation", () => {
    assert.ok(ledger, "module did not load");
    assert.throws(
      () =>
        ledger.parseTransactions(
          "id,account,type,amount\n1,A,debit,1.00\n1,B,credit,1.00\n",
        ),
      /duplicate/i,
    );
    for (const amount of ["1.001", "-1.00", "1e2", "", "abc"]) {
      assert.throws(
        () =>
          ledger.parseTransactions(
            `id,account,type,amount\n1,A,debit,${amount}\n`,
          ),
        `amount ${amount} must be rejected`,
      );
    }
    assert.throws(
      () => ledger.parseTransactions("id,account,type\n1,A,debit\n"),
      /header/i,
    );
  });

  await check("reconciliation-contract", () => {
    assert.ok(ledger, "module did not load");
    const transactions = ledger.parseTransactions(
      "id,account,type,amount\n1,Zed,debit,4.00\n2,Alpha,debit,1.25\n3,Alpha,credit,1.25\n4,Zed,credit,1.50\n5,Middle,credit,2.00\n",
    );
    assert.deepEqual(ledger.reconcileAccounts(transactions), [
      {
        account: "Alpha",
        debitCents: 125,
        creditCents: 125,
        differenceCents: 0,
        status: "balanced",
      },
      {
        account: "Middle",
        debitCents: 0,
        creditCents: 200,
        differenceCents: -200,
        status: "credit_excess",
      },
      {
        account: "Zed",
        debitCents: 400,
        creditCents: 150,
        differenceCents: 250,
        status: "debit_excess",
      },
    ]);
  });

  await check("report-contract", () => {
    assert.ok(ledger, "module did not load");
    const transactions = ledger.parseTransactions(
      "id,account,type,amount\n1,A,debit,1.00\n2,A,credit,1.00\n3,B,credit,2.00\n",
    );
    assert.deepEqual(ledger.renderReport(transactions), {
      summary: {
        accounts: 2,
        balanced: 1,
        unbalanced: 1,
        totalDebitCents: 100,
        totalCreditCents: 300,
      },
      accounts: [
        {
          account: "A",
          debitCents: 100,
          creditCents: 100,
          differenceCents: 0,
          status: "balanced",
        },
        {
          account: "B",
          debitCents: 0,
          creditCents: 200,
          differenceCents: -200,
          status: "credit_excess",
        },
      ],
    });
  });

  await check("protected-files-unchanged", () => {
    for (const relative of ["SPEC.md", path.join("test", "contract.test.js")]) {
      assert.equal(
        hash(path.join(workspace, relative)),
        hash(path.join(seed, relative)),
        `${relative} was modified`,
      );
    }
  });

  if (phase === "full") {
    const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "opentopia-ledger-eval-"));
    try {
      await check("cli-success-contract", () => {
        const input = path.join(tempRoot, "input.csv");
        const output = path.join(tempRoot, "report.json");
        fs.writeFileSync(
          input,
          'id,account,type,amount\n1,"North, Inc.",debit,5.00\n2,"North, Inc.",credit,5.00\n3,West,debit,2.50\n',
        );
        const run = spawnSync(
          process.execPath,
          [path.join(workspace, "src", "cli.js"), "--input", input, "--output", output],
          { encoding: "utf8", cwd: workspace },
        );
        assert.equal(run.status, 0, run.stderr);
        assert.equal(
          run.stdout,
          "Reconciled 2 accounts: 1 balanced, 1 unbalanced.\n",
        );
        const raw = fs.readFileSync(output, "utf8");
        assert.ok(raw.endsWith("\n"), "output must end with a newline");
        const report = JSON.parse(raw);
        assert.equal(report.summary.totalDebitCents, 750);
        assert.equal(report.summary.totalCreditCents, 500);
        assert.deepEqual(
          report.accounts.map((row) => row.account),
          ["North, Inc.", "West"],
        );
      });

      await check("cli-failure-contract", () => {
        const input = path.join(tempRoot, "invalid.csv");
        const output = path.join(tempRoot, "invalid.json");
        fs.writeFileSync(input, "id,account,type,amount\n1,A,debit,1.001\n");
        const run = spawnSync(
          process.execPath,
          [path.join(workspace, "src", "cli.js"), "--input", input, "--output", output],
          { encoding: "utf8", cwd: workspace },
        );
        assert.notEqual(run.status, 0);
        assert.ok(run.stderr.trim().length > 0, "stderr must explain the failure");
        assert.equal(fs.existsSync(output), false, "failed run wrote an output file");
      });
    } finally {
      fs.rmSync(tempRoot, { recursive: true, force: true });
    }
  }

  const passedChecks = checks.filter((item) => item.passed).length;
  const result = {
    schemaVersion: 1,
    phase,
    passed: passedChecks === checks.length,
    passedChecks,
    totalChecks: checks.length,
    checks,
  };
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
  process.exitCode = result.passed ? 0 : 1;
}

main().catch((error) => {
  process.stdout.write(
    `${JSON.stringify({
      schemaVersion: 1,
      phase,
      passed: false,
      passedChecks: 0,
      totalChecks: 1,
      checks: [{ id: "grader-internal", passed: false, detail: error.message }],
    })}\n`,
  );
  process.exitCode = 1;
});
