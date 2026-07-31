# Private Browser Control Suite

This suite drives a real local Chromium instance through the OpenTopia HTTP adapter. Each
trial receives a separate fixture route, while the whole run receives a separate browser
profile and download directory.

Run it from the repository root:

```powershell
.\scripts\evaluate-opentopia-browser-suite.ps1 `
  -EnvFile "J:\path\to\.env" `
  -Profile AUDIT_COPILOT_LLM `
  -Repetitions 1
```

The result directory is `.opentopia/evaluations/<run-id>/`. It contains the fixture backend
state, browser download directory, harness report, and normalized event traces. The fixture is
loopback-only and is stopped when the run ends.

The model sees task prompts and the fixture root URL only. Expected backend state and grader
logic remain outside the trial workspace. Keep candidate variants and release holdouts in a
separate controlled repository when comparing models.
