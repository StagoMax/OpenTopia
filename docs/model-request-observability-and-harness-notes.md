# Model Request Observability and Local Harness Notes

Date: 2026-07-18

This note separates verified request construction from clues found in packaged
applications. A string embedded in a binary proves that a feature or template
exists; it does not prove that every request contains that string.

## OpenTopia request snapshots

Each `AgentCore` provider round now emits a persisted `model_request` event
immediately before calling the provider. The snapshot contains the complete
logical `ModelRequest`:

- `systemPrompt`
- prior `conversation` messages and typed content parts
- the current `userMessage` and native `userContent`
- model-visible `toolCandidates` and their JSON schemas
- prior tool calls and tool results used for continuation

The event is written through the existing SQLite event store and is available
through the existing thread events API and SSE stream. The desktop turn
timeline renders every round as an expandable `Model request #N` entry.

The snapshot intentionally does not contain the authorization header or API
key. It is the provider-neutral logical request, not a byte-for-byte HTTP
capture. The OpenAI-compatible adapter subsequently adds the selected model,
temperature, token limit, reasoning effort, streaming options, and converts the
logical request into Chat Completions `messages` and `tools`. A compatibility
retry after a provider HTTP 400 is not currently emitted as a separate event.

Context-compaction requests made directly by the server do not currently pass
through `AgentCore`, so they are not included in the per-turn request events.

## Local ChatGPT desktop evidence

The application investigated here is the current ChatGPT desktop application,
not the separately installed Codex CLI. Its Windows package keeps the legacy
identity `OpenAI.Codex`, but the live product surface is ChatGPT:

- MSIX: `OpenAI.Codex_26.715.2305.0_x64__2p2nqsd0c76g0`
- manifest display name: `ChatGPT`
- executable: `app\ChatGPT.exe`
- archive metadata: `codexAppBrand: "chatgpt"`, build number `5488`
- bundled desktop agent runtime observed in rollouts: `0.145.0-alpha.18`

The standalone `codex-cli 0.142.5` installation was not used as evidence for
this section.

### Desktop request assembly

Static inspection of the installed `app\resources\app.asar` shows that the
desktop app builds `thread/start` parameters before handing execution to the
bundled agent runtime:

1. It loads config, account/provider data, workspace and Git state.
2. It requests dynamic tools for the selected thread mode.
3. It calls the desktop host operation `developer-instructions` with the
   existing developer instructions, cwd, thread id, host id, instruction
   overrides, and whether thread tools are enabled.
4. The returned text replaces `developer_instructions` in `thread/start`.

The installed builder combines the base developer text with a wrapped
`<app-context>` section and conditionally adds workspace-dependency guidance,
thread-tool guidance, prose-detail guidance, heartbeat instructions, and Git
directives. The default desktop context itself contains these sections:

- Images/Visuals/Files
- Workspace Dependencies
- Automations
- Thread Coordination
- Inline Code Comments
- Git

The app-server then combines those desktop instructions with the agent runtime's
base instructions, world-state fragments, history, current input, and
model-visible tools.

### Verified current desktop rollout

The current task's local rollout is:

```text
~/.codex/sessions/2026/07/18/
rollout-2026-07-18T12-25-20-019f7378-aa2e-7ca3-bdf1-ebbbca5e8214.jsonl
```

Its `session_meta` records `originator: "Codex Desktop"`; the internal
`source: "vscode"` label is retained for compatibility and does not mean this
task came from the standalone CLI. The following model-visible inputs were
observed before the first plain user message:

| Role | Fragment | Size observed |
| --- | --- | ---: |
| base instructions | agent personality and operating contract | 16,299 chars |
| developer | `<permissions instructions>` | 363 chars |
| developer | `<app-context>` | 5,314 chars |
| developer | `<collaboration_mode>` | 977 chars |
| developer | `<plugins_instructions>` | 1,014 chars |
| developer | `<skills_instructions>` | 15,117 chars |
| developer | primary-agent/team coordination instructions | 2,183 chars |
| developer | `<multi_agent_mode>` | 271 chars |
| user | `<environment_context>` | 471 chars |

The rollout's `world_state` also contains separate fields for `agents_md`, app
instructions, environments, environment instructions, host skills, plugin
instructions, and skills. `turn_context` records cwd, workspace roots, date,
timezone, approval policy, sandbox/permission profile, model, reasoning effort,
personality, collaboration mode, and multi-agent mode.

The desktop session registered the dynamic tool `codex_app`; standard built-in
and plugin tool schemas are assembled by the bundled runtime for each inference
request. Subsequent history adds assistant messages, reasoning items, tool
calls, tool outputs, context changes, and later user/developer input.

Therefore the ChatGPT desktop model input is not the composer text alone. It is
the base agent contract plus desktop-specific developer instructions, current
world and turn state, durable repository/user guidance, selected skills and
plugins, conversation/tool history, the current user input, and model-visible
tool schemas. The rollout is a replayable model-context record, but not a
byte-for-byte TLS request capture and it does not contain authorization headers.

## Local Trae evidence

Installed versions observed on this machine:

- Trae CN: `3.3.72`, installed at `J:\Trae CN`
- TRAE Work/SOLO CN: `0.1.36`, installed at
  `D:\Software\TRAE SOLO CN`

Trae's main coding agent is a native module rather than readable JavaScript:

```text
J:\Trae CN\resources\app\modules\ai-agent\ai_agent.dll
D:\Software\TRAE SOLO CN\resources\app\modules\ai-agent\ai_agent.dll
```

The two binaries have different SHA-256 hashes. Their embedded symbols and log
targets identify the following request-building stages and inputs:

- `ChatPromptBuilder`, `ChatPromptBuilderImpl`, and `build_llm_prompt`
- `rs_03_get_history_message`, `rs_06_resolver_user_message`, and
  `rs_13_render_user_prompt`
- `agentName`, `systemPrompt`, `whenToUse` for custom agents
- named system-prompt templates for title, icon, project, branch, pull request,
  input optimization, and custom-agent generation
- model-visible tool calls/results, `custom_tools`, browser/MCP/skill tools, and
  tool-result trimming/compaction controls

The readable workbench layer confirms durable rule injection:

- workspace rules under `.trae/rules/`
- `project_rules.md`
- user rules under the product data `user_rules/` directory and the legacy
  `user_rules.md`
- `AGENTS.md` import enabled by default
- optional `CLAUDE.md` and `CLAUDE.local.md` imports
- rule modes `alwaysApply`, file-specific, model-decision, and manual

Trae's inline-completion subsystem is separate from the agent. Its prompt
template includes edit history, retrieved snippets, symbols, RAG context, file
path, prefix, and suffix around FIM markers. This is evidence for completion
requests, not proof of the chat agent's system prompt.

The local `ai-agent` stdout log exposes stage and route names but contains no
literal `systemPrompt` entry. It shows prompt-building stages, history/user
message resolution, request routing, model-detail lookup, tools, custom tools,
MCP, and tool-result commits. The complete Trae base system prompt cannot be
reliably recovered from these logs; it may be embedded, selected remotely, or
assembled from both local and server-side configuration.

## Harness layers to keep visible

For OpenTopia, request observability should keep these layers distinguishable:

1. Base agent/harness instructions and policy boundaries.
2. Workspace, sandbox, permission, time, and model context.
3. User-level and repository-level durable instructions.
4. Conversation history, compacted summaries, and the current user message.
5. Selected skills, plugins, MCP resources, and other injected context.
6. Tool names, descriptions, JSON schemas, calls, and results.
7. Provider generation settings and the final transport payload.

The implemented logical snapshot covers layers 1 through 6 for `AgentCore`
rounds. Capturing layer 7 and direct server-side model calls requires a provider
transport observer rather than adding secrets to application logs.
