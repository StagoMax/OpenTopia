# Document triage procedure

Implement `classifyDocument(text)` and `extractInvoice(text)` in
`src/documents.js`, plus the CLI in `src/cli.js`.

A document is an invoice only when it contains an `Invoice Number` field and at
least one of `Total`, `Grand Total`, or `Amount Due`. Matching is
case-insensitive. Extract decimal amounts without floating-point arithmetic.
When both Total/Grand Total and Amount Due exist, prefer Grand Total, then Total.
Tax/VAT/GST is optional and defaults to zero. Return integer cents.

`node src/cli.js --input <directory> --output <directory>` must move every
regular `.txt` document into `output/invoices` or `output/other`. Unsupported
files are errors and must not cause a partial move. Write
`output/invoices/summary.csv` with rows sorted by filename and columns:

```text
filename,invoice_number,total_cents,tax_cents
```

Append a `TOTAL,,<sum>,<sum>` row. On success the input directory is empty.
