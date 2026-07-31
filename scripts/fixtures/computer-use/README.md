# Computer Use Fixtures

These fixtures evaluate native Windows desktop control through OpenTopia's `computer` tool.
They are intentionally separate from browser evaluation.

Run them only in a dedicated Windows evaluation account or VM with no unrelated windows,
accounts, credentials, or personal data. The runner automatically approves only `computer`
actions so the agent can operate the fixture; `-IsolatedDesktop` is mandatory to make that
authorization explicit.

```powershell
.\scripts\evaluate-computer-use.ps1 `
  -EnvFile "J:\path\to\.env" `
  -Profile AUDIT_COPILOT_LLM `
  -ExpectedModel glm-5.2 `
  -IsolatedDesktop
```

Each task keeps its fixture process and state file under `.opentopia/evaluations/<run-id>/harness/`,
outside the Agent workspace. A fixture must start in a baseline state that fails its grader,
and must expose a deterministic final state that the external grader can verify.
