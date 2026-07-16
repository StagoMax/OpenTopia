# Ledger Reconciliation Contract

Implement the dependency-free Node.js ledger tool in this workspace.

## Library

`src/ledger.js` must export:

- `parseTransactions(csvText)`: parse UTF-8 CSV with an optional BOM, LF or CRLF,
  quoted fields, escaped quotes, and a required header of
  `id,account,type,amount`. Ignore blank lines. IDs must be unique. Type must be
  `debit` or `credit`. Amount must be a positive decimal with exactly zero, one,
  or two fractional digits and must be converted to integer cents without
  floating-point arithmetic. Reject malformed input with an `Error`.
- `reconcileAccounts(transactions)`: return account rows sorted by account name.
  Each row has `account`, `debitCents`, `creditCents`, `differenceCents`, and
  `status`. Difference is debit minus credit. Status is `balanced`,
  `debit_excess`, or `credit_excess`.
- `renderReport(transactions)`: return `{ summary, accounts }`. Summary has
  `accounts`, `balanced`, `unbalanced`, `totalDebitCents`, and
  `totalCreditCents`.

## CLI

`node src/cli.js --input <csv> --output <json>` must read the CSV, write the
report as pretty JSON with a trailing newline, and print exactly:

`Reconciled N accounts: B balanced, U unbalanced.`

Invalid arguments or data must print a concise error to stderr and exit nonzero.
Do not write an output file after a failed run.

## Constraints

- Use only Node.js built-ins.
- Do not modify this specification or files under `test/`.
- Run `npm test` before declaring a phase complete.
