# Journal recovery incident

The service stores `{ sequence, records }` in a JSON snapshot and appends JSONL
journal frames. Each frame is `{ sequence, operation, checksum }`, where
`checksum` is the lowercase SHA-256 of the compact JSON serialization of
`{ sequence, operation }` with that key order.

Implement `src/journal.js`:

- `parseJournal(text)` ignores one final truncated non-JSON line, but rejects
  malformed lines anywhere else, checksum mismatches, duplicate/out-of-order
  sequence numbers, and unsupported operations.
- `recover(snapshot, frames)` applies only consecutive frames after the
  snapshot sequence. Supported operations are `put` with `{ key, value }` and
  `delete` with `{ key }`. Deleting an absent key is valid.
- Return `{ sequence, records }` with record keys sorted. Do not mutate inputs.

The CLI is:

`node src/cli.js --snapshot <json> --journal <jsonl> --output <json>`

It writes pretty JSON with one trailing newline and prints exactly
`Recovered sequence N with R records.` Invalid input exits nonzero and does not
write an output file.
