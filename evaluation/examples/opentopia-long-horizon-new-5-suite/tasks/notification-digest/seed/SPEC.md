# Notification Digest Contract

Implement the dependency-free Node.js notification digest tool in this workspace.

## Library

`src/digest.js` must export:

- `validateEvents(events)`: validate an array of unique events shaped `{ id, recipient, category, severity, createdAt, read }`. String fields are non-empty. Severity is `info`, `warning`, or `critical`; `read` is boolean; `createdAt` is a valid ISO timestamp. Return normalized copies sorted by `id`, with timestamps canonicalized using `toISOString()`.
- `buildDigests(events, since)`: include only unread events whose timestamp is at or after `since`. Group them by recipient. Return `{ since, digests }`. Digests are sorted by recipient and shaped `{ recipient, critical, warning, info, items }`. Items are `{ id, category, severity, createdAt }`, sorted by severity rank `critical > warning > info`, then ascending timestamp, then `id`.
- `summarizeDigests(result)`: return `{ recipients, notifications, critical, warning, info }`.

Do not mutate caller-owned arrays or objects.

## CLI

`node src/cli.js --input <json> --since <iso> --output <json>` reads `{ "events": [...] }`, writes `{ "result": ..., "summary": ... }` as pretty JSON with a trailing newline, and prints exactly:

`Built R digests with N unread notifications.`

Invalid arguments or data must print a concise error to stderr, exit nonzero, and not write an output file.

## Constraints

- Use only Node.js built-ins.
- Do not modify this specification or files under `test/`.
- Run `npm test` before declaring a phase complete.
