# OpenTopia regression corpus

This directory is the durable memory of evaluation failures. A benchmark run is
temporary evidence; a regression case is the permanent, minimized contract that
prevents the same class of failure from returning.

The registry deliberately contains two kinds of cases:

- `incident`: a failure observed in an eval or real session, with the original
  run/trial/thread identity and a root-cause status.
- `risk`: preventive coverage for a high-risk path such as tool replay,
  reconnect, restart recovery, protected files, or long tool loops.

Every case must link to at least one executable evaluation task or source test.
An `active`, `fixed`, or `monitoring` case must have `regression` coverage. An
`open` incident may temporarily have only a reproducer or monitoring task, but
it must record the next action needed to close the gap.

New Harness results also contain `failureCategory`, `error`, and a structured
`failure` object. This is automatic symptom classification, not a root-cause
claim. Provider transport, adapter, and grader failures are reported separately
and excluded from the model-ability denominator. A human must still inspect the
evidence before recording a confirmed root cause in this registry.

## Workflow after every failure

1. Preserve the raw evidence: run ID, trial ID, Topia thread ID, terminal error,
   grader checks, and redacted transport diagnostics.
2. Separate the failure domains: model behavior, Harness/runtime, provider
   transport, grader, safety boundary, or external infrastructure.
3. Record the symptom and root cause separately. Use `unknown` or `suspected`
   until evidence establishes the cause; do not turn an error string into a
   confident diagnosis.
4. Minimize the failure into the smallest deterministic source test when
   possible. Keep an application-level evaluation task when the behavior depends
   on prompts, tool selection, multiple turns, or recovery state.
5. Link the case in `registry.json`. The registry validator rejects missing
   files, missing test anchors, unknown task IDs, and unknown grader check IDs.
6. Run the appropriate gate and attach the new summary to the report. Move an
   incident to `fixed` only after regression coverage passes; keep it in the
   registry permanently unless the behavior is intentionally retired.

## Gates

| Gate | Purpose | Typical contents |
|---|---|---|
| `smoke` | Seconds; deterministic | schema, parser, grader, tiny tool contracts |
| `pr` | Minutes; deterministic or small model sample | provider faults, cross-tool replay, restart smoke, protected paths |
| `nightly` | Costly and stochastic | full agent tasks, long horizon, prompt/Harness A/B runs |
| `manual` | Human diagnosis | new or insufficiently minimized incidents |

Use one repetition for quick feasibility, three for normal comparisons, and five
for a noisy high-impact decision. Keep provider/model/settings fixed when
comparing Harness or system-prompt changes.

## Commands

Validate all registry links and generate a coverage report:

```powershell
pnpm eval:regressions
```

Attach one or more evaluation summaries to calculate observed pass rates for
the matching cases:

```powershell
node evaluation/src/cli.mjs regressions `
  --registry evaluation/regressions/registry.json `
  --summaries <baseline-summary.json>,<candidate-summary.json> `
  --output .opentopia/evaluations/regressions.md
```

The report separates three numbers that should not be conflated:

- registry coverage: known cases with an executable regression;
- evaluated case coverage: cases exercised by the supplied summaries;
- valid attempt pass rate: passed attempts after excluding grader and
  infrastructure errors.

Run the deterministic Harness tests with:

```powershell
pnpm test:evaluation
```

Run each `source-test` command shown in the generated registry report as part of
the relevant Rust/Node gate. The registry stores the exact command and verifies
that its named test anchor still exists.

## Required case quality

A useful case is specific enough to fail for one reason and broad enough to
catch the same failure class after a refactor. Prefer objective final-state,
trajectory, safety, and protocol checks. Avoid grading prose style unless prose
quality is the actual product requirement.

For agent behavior, keep typical, edge, and adversarial cases. For transport and
Harness behavior, prefer deterministic fault injection: truncated streams,
missing terminal events, malformed tool arguments, disconnects before and after
side effects, duplicate deltas, stale observations, restart ordering, and
partial persistence.
