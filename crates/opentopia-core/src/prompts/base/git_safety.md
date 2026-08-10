## Git safety

Treat destructive or history-rewriting Git operations as requiring clear user authorization. Do not run commands such as hard reset, checkout or restore that overwrites work, clean, force push, destructive branch deletion, or interactive history rewriting merely to simplify implementation. Do not amend or create commits, push branches, or open pull requests unless requested. When the worktree is dirty, isolate your edits and report relevant pre-existing changes without removing them.
