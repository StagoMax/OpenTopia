import assert from "node:assert/strict";
import test from "node:test";

import {
  parseTransactions,
  reconcileAccounts,
  renderReport,
} from "../src/ledger.js";

test("parses valid rows into integer cents", () => {
  assert.deepEqual(
    parseTransactions(
      "id,account,type,amount\n1,Alpha,debit,10.25\n2,Alpha,credit,10.25\n",
    ),
    [
      { id: "1", account: "Alpha", type: "debit", amountCents: 1025 },
      { id: "2", account: "Alpha", type: "credit", amountCents: 1025 },
    ],
  );
});

test("rejects duplicate transaction IDs", () => {
  assert.throws(
    () =>
      parseTransactions(
        "id,account,type,amount\n1,Alpha,debit,1.00\n1,Beta,credit,1.00\n",
      ),
    /duplicate/i,
  );
});

test("reconciles and sorts accounts", () => {
  const rows = reconcileAccounts([
    { id: "1", account: "Zulu", type: "credit", amountCents: 300 },
    { id: "2", account: "Alpha", type: "debit", amountCents: 500 },
    { id: "3", account: "Alpha", type: "credit", amountCents: 200 },
  ]);
  assert.deepEqual(rows, [
    {
      account: "Alpha",
      debitCents: 500,
      creditCents: 200,
      differenceCents: 300,
      status: "debit_excess",
    },
    {
      account: "Zulu",
      debitCents: 0,
      creditCents: 300,
      differenceCents: -300,
      status: "credit_excess",
    },
  ]);
});

test("renders summary totals", () => {
  const transactions = parseTransactions(
    "id,account,type,amount\n1,A,debit,1\n2,A,credit,1.00\n3,B,debit,2.5\n",
  );
  assert.deepEqual(renderReport(transactions).summary, {
    accounts: 2,
    balanced: 1,
    unbalanced: 1,
    totalDebitCents: 350,
    totalCreditCents: 100,
  });
});
