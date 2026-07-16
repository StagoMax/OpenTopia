# GLM-5.2 Long-Horizon Evaluation (2026-07-16)

## Scope

This is an OpenTopia-local, deterministic long-horizon evaluation. It is not an
official SWE-bench or Terminal-Bench score and must not be compared with their
leaderboards. Docker-backed official harnesses remain outside the current local
MVP scope.

The design follows two authoritative benchmark patterns:

- [SWE-bench](https://github.com/SWE-bench/SWE-bench): a real repository and issue
  are resolved by an agent and graded by deterministic tests. Its methodology is
  described in the [ICLR 2024 paper](https://openreview.net/forum?id=VTF8yNQM66).
- [Terminal-Bench 3](https://github.com/harbor-framework/terminal-bench-3): realistic
  terminal tasks use isolated environments, oracle-checked tasks, and objective
  test scripts. Its benchmark paper is available on
  [OpenReview](https://openreview.net/forum?id=a7Qa4CcHak).

## Protocol

The runner is `scripts/evaluate-long-horizon.ps1`. It creates a fresh Git fixture
from `scripts/fixtures/long-horizon/seed`, verifies that the baseline fails, and
uses an external hidden grader that the agent cannot edit.

1. Probe the configured OpenAI-compatible provider: model listing, SSE text,
   automatic tool calls, one-call continuation, serialized history diagnostics,
   and compacted-history compatibility.
2. Phase 1 asks the agent to implement the CSV ledger library and run public tests.
3. A successful Phase 1 would restart OpenTopia Server against the same SQLite DB.
4. Phase 2 would recover the durable thread/plan and implement the CLI.
5. Grade protected-file hashes, six library checks, two CLI checks, plan/test
   process requirements, restart recovery, and a byte-level secret scan.
6. Cancel a phase after 420 seconds. A timeout is a failure, even if some tests pass.

The API key is read from `J:\Project\信贷审核助手\.env` only in the process that
runs the probe/evaluation. The committed report contains no key. Desktop storage
uses Electron `safeStorage`; the renderer can read metadata only.

## Result

Machine-readable result:
`docs/evaluations/glm-5.2-long-horizon-2026-07-16.json`.

| Metric | Result |
| --- | --- |
| Run | `glm-5-2-20260716T075801Z` |
| Overall | **failed** |
| Provider model/list/text/tool/continuation | passed |
| Strict serialized multi-tool history | HTTP 400 from external gateway |
| Compacted tool-history compatibility | HTTP 200 |
| Baseline library/full | 2/6 and 3/8, expected failure |
| Final library grader | 6/6 passed |
| Final full grader | 7/8 passed |
| Missing check | CLI success contract |
| Tool calls | 42 started, 42 finished |
| Execution slices | 4 (`turn_started` events) |
| Budget checkpoints | 3 |
| Plan updates | 5 |
| Test command calls | 6 |
| Tokens | 442,160 input; 25,267 output; 467,427 total |
| Elapsed | 441,958 ms |
| Secret scan | passed |
| Terminal failure | Phase 1 exceeded the 420-second hard timeout |

The agent produced a working ledger library and passed all hidden library checks,
but it repeatedly read files, updated the plan, and ran tests instead of ending
Phase 1 after the scoped objective was satisfied. It never reached the restart and
CLI phase. This is evidence of useful coding ability, but not a reliable long-task
closure yet.

## Engineering Conclusions

1. Provider protocol compatibility is no longer adequately tested by one chat
   request. The probe now includes tool-result continuation and compacted history.
2. The agent needs an explicit phase-completion controller that can stop a slice
   when required tests pass and the scoped plan steps are complete.
3. Repeated reads and test commands need trajectory-level deduplication or a tool
   repetition budget. The current run spent 467k cumulative tokens on a small task.
4. Compacted provider history should become bounded and incremental rather than
   repeatedly transmitting all prior tool output.
5. Restart recovery remains unmeasured in this run because Phase 1 did not finish;
   it must remain an open acceptance item.

## Reproduce

```powershell
powershell -ExecutionPolicy Bypass -File scripts/evaluate-long-horizon.ps1 `
  -EnvFile "J:\Project\信贷审核助手\.env" `
  -Profile AUDIT_COPILOT_LLM `
  -ExpectedModel glm-5.2 `
  -SummaryPath "docs\evaluations\glm-5.2-long-horizon-2026-07-16.json"
```

Evaluation output under `.opentopia/evaluations/` is intentionally ignored. It
contains local databases, trajectories, logs, and workspaces for debugging.
