# Architecture Evaluation Protocol

OpenTopia uses different evaluation layers for different decisions. A single
score should not be used as both a fast development signal and an external
claim.

## Layers

1. **Smoke and contract tests** catch runner, adapter, grader, and telemetry
   failures before model time is spent.
2. **Frozen conformance suites** preserve the original long-horizon contracts.
   Their task content must not change in place.
3. **Architecture Calibration v1** is the 12-task public suite used for broad,
   paired comparisons of prompts and Harness behavior.
4. **Sealed holdouts** must live outside the repository and use
   `visibility: "sealed"`. Rotate them after exposure; a task committed here is
   no longer a sealed holdout.
5. **External benchmarks** are milestone checks. Run Terminal-Bench through
   Harbor for terminal-agent behavior, then SWE-bench-style repository tasks
   once the architecture and patch workflow are stable.

## Paired experiment rule

Each baseline/candidate pair uses the same suite, task hashes, target, model,
provider profile, reasoning effort, budgets, environment, and repetitions.
Those values belong in `controlled`. System-prompt version/hash, Harness
revision, runtime policy, or one feature flag belongs in `treatment`.

Use a new `pairingKey` for a new controlled setup. Do not compare tagged and
untagged runs. The comparison command rejects changed controlled factors and
changed suite, target, or task hashes.

Change one treatment at a time when attribution matters. A prompt plus runtime
change is still a valid combined treatment, but it cannot show which component
caused the result.

## Decision cadence

- One repetition: quick architecture feasibility and breakage discovery.
- Three repetitions: routine prompt or Harness A/B decision.
- Five repetitions: promotion check for noisy or high-impact changes.
- External benchmark: after an internal candidate clears correctness and
  safety gates.

Primary gates are strict success, per-task success, outcome, safety,
trajectory, and controlled-restart recovery. Secondary metrics are provider and
uncached tokens, estimated cost, latency, tool calls, finalization rejects,
invalid-tool loops, no-progress signals, and duplicate plans.

Never rewrite a frozen task to make a candidate pass. Correct an invalid task
by publishing a new suite version and keeping the old result identifiable.
