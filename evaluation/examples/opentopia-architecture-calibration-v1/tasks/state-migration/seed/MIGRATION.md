# State migration v1 to v3

Implement the Python module `migration.py` without third-party packages.

The legacy directory contains `users.json` rows `{id, name, email}` and
`sessions.json` rows `{token, user_id, expires_at}`. IDs may contain uppercase
letters. The v3 output contains:

- `accounts.json`: `{account_id, display_name, normalized_email}`
- `sessions.json`: `{token_hash, account_id, expires_at}`
- `manifest.json`: counts plus `source_sha256`, the SHA-256 over the raw
  `users.json` bytes followed by the raw `sessions.json` bytes.

Account IDs are lowercase legacy IDs. Emails are trimmed and lowercased.
`token_hash` is lowercase SHA-256 of the UTF-8 token. Canonicalize timestamps
to UTC `Z`. Reject normalized ID/email collisions, invalid timestamps, unknown
user references, duplicate tokens, and unknown fields.

`plan_migration(source_dir)` returns the complete output plus source hash.
`apply_migration(plan, output_dir, source_dir)` verifies the source hash and
atomically replaces output. CLI modes are `--dry-run` and `--apply`; dry-run
writes a plan JSON but no output directory.
