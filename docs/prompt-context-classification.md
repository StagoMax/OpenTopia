# Prompt Context Classification

OpenTopia classifies model context along independent axes. Provider message
roles, semantic authority, semantic lifetime, and cache placement must not be
treated as one enum or rendered as interchangeable prompt labels.

## Axes

`ContextRole` is the provider transport role: `system`, `developer`, `user`,
`assistant`, or `tool`.

`ContextAuthority` records what the harness believes the content can do:

- `system`: product-wide invariant policy;
- `developer`: trusted application, repository, Agent, or selected Skill policy;
- `user`, `assistant`, and `tool`: content retaining its original speaker;
- `data`: asserted context or state that does not mint new instructions.

`ContextLifecycle` is internal audit and invalidation metadata:

- `build`: fixed until the prompt build changes;
- `thread`: fixed for the thread configuration;
- `epoch`: fixed until a checkpoint or lineage boundary changes;
- `turn`: belongs to the current user Turn;
- `round`: belongs to one model/tool Round.

`ContextCacheScope` controls internal ordering and provider-cache placement. It
is not a lifecycle and is never emitted as a provider prompt tag. For example,
a Skill selected for one Turn has `lifecycle=turn`, but may use the thread
prefix cache segment so every provider sees the instruction before replayed
history. A durable checkpoint has `lifecycle=epoch` even when placed in that
same prefix.

## Required classifications

| Content | Authority | Lifecycle |
| --- | --- | --- |
| Base prompt | system | build |
| Runtime, permission, experience, and output policy | developer | thread |
| `AGENTS.md` | developer | thread |
| Agent template and collaboration policy | developer | thread |
| Plugin and Skill catalogs | data | thread |
| Selected Skill instructions | developer | turn |
| Durable checkpoint or summary | data | epoch |
| Current user request and attachment manifest | user | turn |
| Dynamic world state | data | turn |
| Tool call and result | assistant/tool | round |

Only explicitly authorized loaders may create developer policy. Ordinary files,
attachments, tool observations, generated summaries, and capability catalogs
remain data even when a provider requires a developer-shaped envelope to carry
them.

## Provider mapping and tools

Provider adapters consume `ContextRole`, message order, and cache settings. They
do not receive `ContextAuthority` or `ContextLifecycle` as native fields or
prompt tags. Data carried in an instruction-capable transport role is wrapped
as `context_data` and explicitly described as context rather than instructions.

Tool definitions are not copied into textual context items. The Agent builds
`ModelRequest.tool_candidates`, and adapters map those candidates to native
`tools` or Codex `dynamicTools`. Direct tool schemas, deferred tool catalogs,
and schemas loaded after Tool Search are estimated separately in
`TokenEstimateBreakdown`. Tool calls and tool results retain their native
assistant/tool protocol items.
