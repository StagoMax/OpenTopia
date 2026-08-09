# Retry Scheduler Contract

Implement the dependency-free Node.js retry scheduling tool in this workspace.

## Library

`src/retry.js` must export:

- `validateJobs(jobs)`: validate an array of unique jobs shaped `{ id, attempt, maxAttempts, baseDelayMs, lastFailureAt }`. IDs are non-empty strings. `attempt` is a non-negative integer, `maxAttempts` is a positive integer, `baseDelayMs` is a positive integer, and `lastFailureAt` is a valid ISO timestamp. Return normalized copies sorted by `id`, with timestamps canonicalized using `toISOString()`.
- `planRetries(jobs, now)`: `now` is a valid date or timestamp. A job is `exhausted` when `attempt >= maxAttempts`; it has `delayMs: null` and `nextAttemptAt: null`. Otherwise delay is `min(baseDelayMs * 2 ** attempt, 3600000)`. The state is `ready` when the next attempt time is at or before `now`, otherwise `waiting`. Return `{ now, jobs }`; non-exhausted jobs are sorted by next attempt time then `id`, followed by exhausted jobs sorted by `id`. Each output job is `{ id, attempt, maxAttempts, state, delayMs, nextAttemptAt }`.
- `summarizeRetries(plan)`: return `{ jobs, ready, waiting, exhausted, nextWakeAt }`, where `nextWakeAt` is the earliest waiting timestamp or `null`.

Do not mutate caller-owned arrays or objects.

## CLI

`node src/cli.js --input <json> --now <iso> --output <json>` reads `{ "jobs": [...] }`, writes `{ "plan": ..., "summary": ... }` as pretty JSON with a trailing newline, and prints exactly:

`Scheduled N jobs: R ready, W waiting, E exhausted.`

Invalid arguments or data must print a concise error to stderr, exit nonzero, and not write an output file.

## Constraints

- Use only Node.js built-ins.
- Do not modify this specification or files under `test/`.
- Run `npm test` before declaring a phase complete.
