# OpenTopia Architecture Calibration v1

This is a public, frozen calibration suite for paired OpenTopia architecture
experiments. It is not an official SWE-bench, Terminal-Bench, Harbor, or
tau-bench result, and because its task definitions live in this repository it
must not be described as a sealed holdout.

The tasks are original. Their evaluation shapes are informed by:

- SWE-bench: repair existing behavior and preserve regressions.
- Terminal-Bench: grade the final filesystem, process, service, or data state.
- tau-bench: apply policy rules without violating state-transition constraints.

The suite intentionally mixes four task shapes:

1. Existing-code bug fixes with incomplete issue reports.
2. Terminal-style artifact and local-service outcomes.
3. Policy/state-machine work with forbidden side effects.
4. Two-session recovery tasks with a controlled server restart.

| Family | Tasks |
|---|---:|
| Repository repair | router, async pool, safe paths, workspace build, plugin registry |
| Terminal/final state | journal recovery, static deploy, document triage, log stream, state migration |
| Policy and durability | refund policy, durable queue |

Validate the frozen definition before a run:

```powershell
node evaluation/src/cli.mjs validate `
  --suite evaluation/examples/opentopia-architecture-calibration-v1/suite.json `
  --target evaluation/examples/opentopia-architecture-calibration-v1/target.json
```

Use an experiment profile for paired runs. Put model/provider/budget inputs in
`controlled` and put system-prompt hashes, runtime flags, or Harness revisions
in `treatment`.

```json
{
  "schemaVersion": 1,
  "experimentId": "finalization-guard-2026-08",
  "pairingKey": "architecture-v1-glm52-r3",
  "variant": "baseline",
  "controlled": {
    "model": "glm-5.2",
    "reasoningEffort": "high",
    "repetitions": 3
  },
  "treatment": {
    "systemPromptVersion": "2026-08-01.1",
    "systemPromptSha256": "...",
    "harnessRevision": "..."
  }
}
```

The PowerShell product runner can generate this profile automatically with
`-ExperimentId`, `-PairingKey`, `-Variant`, and `-TreatmentLabel`.

Use the same pairing key and controlled settings for both sides:

```powershell
pwsh -File scripts/evaluate-opentopia-tool-suite.ps1 `
  -EnvFile <provider.env> `
  -SuitePath evaluation/examples/opentopia-architecture-calibration-v1/suite.json `
  -Repetitions 3 `
  -ExperimentId system-prompt-v2 `
  -PairingKey architecture-v1-model-effort-r3 `
  -Variant baseline `
  -TreatmentLabel prompt-v1

# Apply exactly one prompt or Harness change, then repeat with:
# -Variant candidate -TreatmentLabel prompt-v2
```

Compare the two generated harness summaries:

```powershell
node evaluation/src/cli.mjs compare `
  --baseline <baseline-summary.json> `
  --candidate <candidate-summary.json> `
  --output <comparison-directory>
```

Use one repetition for a quick feasibility check, three for routine A/B
decisions, and five before adopting a noisy or high-impact change. If the
system prompt and Harness implementation both change, label that as a combined
treatment; use separate pairs when causal attribution matters.
