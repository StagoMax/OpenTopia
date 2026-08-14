# Agent runtime boundaries

This note records the compatibility-first boundary split around `AgentCore`.
It describes ownership; it does not introduce a new turn protocol or change
existing approval, persistence, tool-ordering, or completion behavior.

## Runtime shape

```text
HTTP / SSE / session lifecycle
            |
            v
    AgentTurnDriver
      |     |     |
      |     |     +-- resume after structured user input
      |     +-------- resume after approval
      +-------------- start a turn
            |
            v
        AgentCore
      /     |      \
continuation  scheduler  completion guard
```

`AgentCore` remains the configured, trusted turn kernel. The server may build
and configure a concrete core, but it starts and resumes execution through the
object-safe `AgentTurnDriver` interface. Product lifecycle code therefore no
longer depends on the names of the concrete loop entry methods.

## Boundary ownership

| Boundary | Owner | Invariants |
| --- | --- | --- |
| Turn entry and resume | `agent_runtime::AgentTurnDriver` | New turns, approval resumes, and structured-input resumes expose one stable execution interface. |
| Correctness continuation | `agent::continuation` | Captures enough state to resume the same turn without replaying committed model or tool work. This is distinct from an optional provider conversation cursor. |
| Tool scheduling | `agent::tool_scheduler` | Preserves provider result order, resource-conflict rules, approval barriers, sandbox policy, maximum concurrency, and turn-scoped path leases. |
| Completion readiness | `agent::completion_guard` | Treats model completion as a candidate; pending actions, approvals, plan/evidence obligations, and active descendants can still block finalization. |
| Loop orchestration | `agent.rs` | Owns the model/action loop and composes the boundaries above in the existing order. |

## Compatibility rules

- `AgentCore` still implements all existing public methods; the driver delegates
  to them, so downstream callers are not forced to migrate immediately.
- Continuation serialization retains the same field names, defaults, and enum
  tags. Existing stored continuations remain readable.
- Scheduler extraction does not change which calls may run concurrently or when
  an approval is required.
- Completion-guard extraction does not change blockers, retry limits, synthetic
  observations, or emitted warning events.
- A provider cursor remains a disposable cross-turn optimization. It must never
  replace the correctness continuation used at an interactive boundary.

## Deliberate non-goals

This split does not make the safety-critical loop dynamically pluggable and it
does not add a root-turn next-step inbox. Those changes require an explicit
protocol for monotonic capability narrowing, event ordering, persistence, and
resume semantics; they should build on these boundaries rather than bypass
them.
