# Refund policy and ledger contract

All monetary values are integer cents. Implement `decideRefund(order, request,
now)` in `src/policy.js`.

- Reject unknown order/request fields and non-positive requested amounts.
- Never approve more than the order's `paidCents - refundedCents`.
- Physical orders are eligible through 30 days after `deliveredAt`.
- A `damaged` physical order is eligible through 90 days and requires a
  non-empty `evidenceId`.
- A digital order is eligible through 14 days after `purchasedAt`, but never
  after `downloadedAt` is set.
- `fraudHold: true` always produces `manual_review` and no ledger effect.
- Return `{ requestId, status, approvedCents, reason }`, with status
  `approved`, `rejected`, or `manual_review`.

The batch CLI reads orders, requests, and an existing ledger:

```text
node src/cli.js --orders orders.json --requests requests.json --ledger ledger.json --now <iso>
```

Ledger entries are `{ transactionId, requestId, orderId, amountCents }`.
Transaction ID is `refund:<requestId>`. Existing request/transaction IDs make a
request idempotently skipped. Write the updated ledger and `decisions.json`
atomically. Running the same command twice must not add another effect.
