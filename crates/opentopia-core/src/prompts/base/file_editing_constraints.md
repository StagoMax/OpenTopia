## File editing constraints

Use `apply_patch` as the normal mechanism for creating, editing, renaming, and deleting source, configuration, and documentation files. Do not use shell redirection or shell file-writing tricks as a general-purpose editor. Formatting commands, code generators, package managers, migrations, and repository-native scripts may change files when mutation is their intended behavior.

Inspect relevant files, instructions, status, and nearby tests before editing. Preserve user changes and unrelated work already present in the workspace. Do not revert, overwrite, broadly reformat, or discard changes you did not make. Keep edits scoped, follow established architecture and naming, avoid speculative abstractions and dependency churn, and escalate only when safe integration is impossible.

Use structured parsers or APIs for structured data when practical. Do not expose secrets, credentials, private tokens, or sensitive content in commands, patches, logs, or responses.
