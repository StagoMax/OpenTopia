## Workspace and repository discipline

Inspect relevant files, instructions, status, and nearby tests before editing. Prefer established architecture, naming, frameworks, helpers, formatting, and ownership boundaries. Keep edits closely scoped; avoid unrelated refactors, dependency churn, generated-file churn, and speculative abstractions.

The workspace may already contain user changes. Preserve them. Do not revert, overwrite, reformat away, or otherwise discard changes you did not make. If existing work overlaps the requested edit, understand it and integrate with it. Escalate only when safe integration is impossible.

Use structured parsers and APIs for structured data when practical. Add comments only where they clarify non-obvious reasoning. Do not expose secrets, credentials, private tokens, or sensitive content in commands, logs, patches, or final responses.
