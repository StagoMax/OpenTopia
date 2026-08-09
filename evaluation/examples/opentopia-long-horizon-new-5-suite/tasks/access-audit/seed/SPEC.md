# Access Policy Audit Contract

Implement the dependency-free Node.js access audit tool in this workspace.

## Library

`src/access.js` must export:

- `validateGrants(grants)`: validate an array of unique grants shaped `{ id, user, resource, role, source, expiresAt }`. String fields are non-empty. Role is `viewer`, `editor`, or `admin`. Source is `direct` or `group`. `expiresAt` is either `null` or a valid ISO timestamp. Return normalized copies sorted by `id`, with timestamps canonicalized using `toISOString()`.
- `auditAccess(grants, now)`: a grant is expired when `expiresAt` is at or before `now`. For each active `user` and `resource` pair, choose one effective grant using role rank `admin > editor > viewer`, then source rank `direct > group`, then ascending grant `id`. Return `{ now, effective, expired, shadowed }`. Effective rows are `{ user, resource, role, source, grantId }` sorted by user then resource. Expired rows are `{ grantId, user, resource, expiredAt }` sorted by `expiredAt` then `grantId`. Every active non-effective grant appears in `shadowed` as `{ grantId, effectiveGrantId, user, resource }`, sorted by `grantId`.
- `summarizeAudit(audit)`: return `{ grants, effective, expired, shadowed, adminAccess }`, where `grants` is the total represented by the other three classifications and `adminAccess` counts effective admin rows.

Do not mutate caller-owned arrays or objects.

## CLI

`node src/cli.js --input <json> --now <iso> --output <json>` reads `{ "grants": [...] }`, writes `{ "audit": ..., "summary": ... }` as pretty JSON with a trailing newline, and prints exactly:

`Audited N grants: E effective, X expired, S shadowed.`

Invalid arguments or data must print a concise error to stderr, exit nonzero, and not write an output file.

## Constraints

- Use only Node.js built-ins.
- Do not modify this specification or files under `test/`.
- Run `npm test` before declaring a phase complete.
