# OpenTopia MVP Implementation

This MVP is the first executable slice of the architecture described in `docs/ai-coding-work-agent-architecture.md`.

The concrete source borrowing map is maintained in `docs/source-adaptation-map.md`.
The remaining implementation slices are tracked in `docs/implementation-backlog.md`.
Product delivery order is tracked in GitHub
[Roadmap #4](https://github.com/StagoMax/OpenTopia/issues/4). The local MVP for
[#3 Project/Thread model](https://github.com/StagoMax/OpenTopia/issues/3) is complete;
[#1 attachments/sources/Skills context](https://github.com/StagoMax/OpenTopia/issues/1),
and [#2 subagent runtime](https://github.com/StagoMax/OpenTopia/issues/2) retain
their multimodal, approval, and resource-budget follow-up work.

## What Exists

### Rust Core

Path: `crates/opentopia-core`

Implemented:

- Domain model: thread, message, message part, tool call, tool result, agent event.
- SQLite session store, including persistent approval records.
- Artifact model and SQLite artifact index with inline/path storage.
- Basic permission policy with configurable command rules and a network policy placeholder.
- Built-in tool abstraction.
- Built-in `list_files`, `read_file`, `write_file`, `search`, `shell`, `git_diff`, and `apply_patch` tools.
- OpenAI-compatible provider with mock fallback.
- Settings, MCP, sandbox, and workspace workbench shared types.
- Context summary, compaction, and provider-reported token-usage event types used by
  the active context-budget runtime.
- Deterministic command parser for `/list`, `/read`, `/search`, `/write`, `/run`, `/diff`, `/patch`.
- Explicit MCP command parser for `/mcp server__tool {json}` after MCP tools are synced into the agent registry.
- `ExecutionEnvironment` trait and `LocalExecutionEnvironment` with file read/write/exec/apply_patch.
- `McpStdioClient` with full stdio process lifecycle: spawn, initialize, list_tools, call_tool, shutdown, timeout handling, stderr logging, and JSON-RPC message parsing.
- `McpExtensionHost` with tool schema caching, public-name routing, and duplicate detection.
- Descriptor/annotation-aware MCP policy checks using permission labels such as `read`, `write`, `network`, `secret`, `destructive`, and `unknown`.

The agent loop currently:

1. Emits `turn_started`.
2. Emits a small model delta.
3. Executes deterministic local tool commands when the user uses slash commands.
4. Otherwise calls the configured OpenAI-compatible provider, falling back to the mock provider.
5. Parses provider `tool_calls`, executes built-in or enabled MCP tools through policy checks, and returns tool results to the provider until the provider reaches a terminal response.
6. Emits tool start/finish events and automatically compacts older completed tool history near the context-window boundary without imposing a task-level round or elapsed-time limit.
7. Emits an assistant message and `turn_finished`.

### Rust Server

Path: `crates/opentopia-server`

Implemented:

- `GET /health`
- `GET /api/settings`
- `PATCH /api/settings`
- `GET /api/provider/health`
- `POST /api/provider/test`
- `GET /api/threads`
- `POST /api/threads`
- `PATCH /api/threads/{thread_id}`
- `DELETE /api/threads/{thread_id}`
- `GET /api/projects`
- `POST /api/projects`
- `PATCH /api/projects/{project_id}`
- `DELETE /api/projects/{project_id}`
- `GET /api/skills?workspaceRoot=...`
- `GET /api/threads/{thread_id}/messages`
- `POST /api/threads/{thread_id}/messages`
- `GET /api/threads/{thread_id}/events`
- `GET /api/threads/{thread_id}/events/stream`
- `GET /api/threads/{thread_id}/turn`
- `POST /api/threads/{thread_id}/turn/cancel`
- `GET /api/threads/{thread_id}/subagents`
- `POST /api/threads/{thread_id}/subagents`
- `POST /api/threads/{thread_id}/subagents/{run_id}/input`
- `POST /api/threads/{thread_id}/subagents/{run_id}/cancel`
- `POST /api/threads/{thread_id}/subagents/{run_id}/wait`
- `GET /api/threads/{thread_id}/workspace/tree`
- `GET /api/threads/{thread_id}/workspace/file`
- `GET /api/threads/{thread_id}/workspace/diff`
- `POST /api/threads/{thread_id}/workspace/diff/revert`
- `POST /api/threads/{thread_id}/workspace/diff/hunk`
- `GET /api/threads/{thread_id}/sandbox`
- `GET /api/threads/{thread_id}/trajectory`
- `GET /api/threads/{thread_id}/artifacts`
- `GET /api/threads/{thread_id}/artifacts/{artifact_id}`
- `POST /api/threads/{thread_id}/previews/resolve`
- `GET /api/threads/{thread_id}/previews/{preview_id}/content`
- `GET /api/threads/{thread_id}/previews/{preview_id}/workbook`
- `GET /api/threads/{thread_id}/previews/{preview_id}/range`
- `GET /api/threads/{thread_id}/context`
- `POST /api/threads/{thread_id}/context/compact`
- `POST /api/threads/{thread_id}/git`
- `POST /api/threads/{thread_id}/terminal/commands`
- `POST /api/threads/{thread_id}/terminal/cancel`
- `GET /api/threads/{thread_id}/terminal/history`
- `GET /api/threads/{thread_id}/terminal/stream`
- `GET|POST /api/threads/{thread_id}/terminal/session`
- `POST /api/threads/{thread_id}/terminal/session/input`
- `POST /api/threads/{thread_id}/terminal/session/resize`
- `POST /api/threads/{thread_id}/terminal/session/close`
- `GET /api/threads/{thread_id}/approvals?status=pending`
- `POST /api/threads/{thread_id}/approvals/{approval_id}/decision`
- `GET /api/mcp/servers`
- `POST /api/mcp/servers`
- `PATCH /api/mcp/servers/{server_id}`
- `DELETE /api/mcp/servers/{server_id}`
- `POST /api/mcp/servers/{server_id}/restart`
- `GET /api/mcp/servers/{server_id}/tools`
- `POST /api/mcp/servers/{server_id}/call-tool`
- `GET /api/threads/{thread_id}/mcp`
- `PUT /api/threads/{thread_id}/mcp/{server_id}`

Events are persisted in SQLite and also broadcast through SSE.
Approval requests are stored in SQLite with `pending`, `approved`, or `denied` status so unresolved requests can be recovered after a server restart.
Settings and MCP server configurations are persisted in SQLite. MCP stdio servers
run through the configured OS sandbox, receive only a minimal environment plus
explicit `envKeys`, refresh `tools/list_changed`, and expose tools only to Threads
where the server is enabled. Enabled servers are restored on startup; create, update,
Thread enablement, and first Agent use all converge on an idempotent lifecycle guard.
Direct calls require a Thread and use its policy/timeline.
Artifacts are indexed in SQLite and trajectory export includes artifact metadata.
Large read/search/shell outputs can surface artifact metadata for UI retrieval.
Context compaction calls the active OpenAI-compatible provider, records durable
summary metadata, and injects the latest summary into later model turns. An
explicit manual summary remains supported. Provider streaming usage is parsed into
persisted `token_usage` events; automatic compaction separately uses the configured
context-window estimate and threshold before bounded-history trimming.

### Rust CLI

Path: `crates/opentopia-cli`

Implemented:

- `opentopia threads`
- `opentopia new`
- `opentopia send <thread_id> <content>`

### Electron Desktop

Path: `apps/desktop`

Implemented:

- Electron shell with context isolation and preload.
- React workbench.
- Thread list.
- Message stream.
- Composer.
- Event timeline.
- Workspace inspector.
- Tool output inspector.
- Approval-needed notification card.
- Allow-once and deny approval actions.
- Offline state when the local server is unavailable.
- Electron development shell can auto-start the local Rust server.
- Workspace picker, recent workspace list, normalized path bridge, and
  open-path action through the preload API.
- Settings modal for provider base URL, model, API-key env name, and permission mode.
- Workbench panels for file tree, first-class read-only preview tabs, git diff,
  MCP extension create/edit/restart/delete and per-Thread enablement, and local sandbox status.
- Xterm-based per-command terminal streaming with cancel.
- SQLite-backed terminal history and cancellation/timeout process-tree cleanup.
- Artifact gallery, artifact preview tabs, context status, explicit-confirm file revert,
  and staged/unstaged hunk stage/unstage/discard.
- First-class SQLite Project/Thread ownership, normalized workspace uniqueness,
  archive/delete/rename/pin flows, and migration from the former renderer-only projection.
- Explicit file/image/document source selection through Electron, server-side canonicalization,
  sensitive/type/size limits, message-persisted references, bounded text context, and right-rail recovery.
- Turn-scoped Codex-compatible Skills discovery/selection and bounded `SKILL.md` injection.
- Real persistent subagents with AgentCore execution, concurrency/depth controls, no scheduler
  execution deadline,
  recursive cancellation, model-callable lifecycle tools, concurrent `wait_agents`, HTTP
  controls, SSE, and right-rail UI.
- Persisted main-Turn state with explicit terminal outcomes, startup interruption recovery,
  race-free SSE replay, and desktop restoration from the stored Turn record.
- `update_plan` durable task memory, restored into later Turns and rendered in the message view.
- A shared per-Thread browser session: Electron uses a sandboxed `WebContentsView` for the
  visible page and exposes a random-token loopback broker to the Rust `DesktopBrowserRuntime`.
  The user and Agent therefore operate on the same page. Pure web mode retains the isolated
  CDP runtime and `BrowserPanel` fallback. Navigation/snapshot/click/type/wait/screenshot/
  download, domain approval continuation, and provider-native screenshot input are implemented.
- First-class preview tabs for workspace files and Thread artifacts. The preview service
  canonicalizes workspace paths, scopes artifacts to the active Thread, applies type/size
  limits, and serves authenticated binary content with revision metadata. Monaco renders
  text/code, Blob URLs render images, PDF.js renders PDF pages, and a virtualized range grid
  renders XLSX sheets. Unsupported formats can be opened with the system application.
- Built-in bounded XLSX inspect/list/read/create/update backed by `calamine` and
  `rust_xlsxwriter`; workspace reads/writes still pass through policy and ExecutionEnvironment.
- Sandboxed Git workflow API for branch/status/commit/push/compare/worktree actions,
  plus desktop status/branch/create/switch/commit/push/compare controls. Worktree UI,
  PR creation, and GitHub CLI remain follow-up work.
- Electron `safeStorage` provider API-key storage: renderer can set/delete the
  provider key and list metadata only; the secret value stays in the main
  process and is injected into the spawned Rust server as `OPENTOPIA_API_KEY`
  only when no explicit env or `.env` key is already present.
- Keyring availability metadata for settings/platform surfaces, including
  safeStorage availability, selected backend when Electron exposes it, and the
  non-secret storage path under Electron `userData`.
- Settings UI for write-only/delete-only desktop API-key management and provider connectivity tests.
- Strict OpenAI-compatible continuation fallback: standard tool messages are tried
  first; a tool-history HTTP 400 is retried once with compacted text history. The
  provider probe covers text SSE, tool calls, continuation, and fallback behavior.
- Deterministic two-phase long-horizon evaluation fixture with baseline failure,
  protected files, hidden grading, restart recovery checks, terminal-state waiting, trajectory
  metrics, and byte-level secret scanning. The 2026-07-16 GLM-5.2 run is documented
  under `docs/evaluations/` and currently fails overall at phase closure.
- Desktop packaging skeleton: `scripts/build-desktop.ps1` builds the release
  Rust server, stages it in `apps/desktop/resources`, and electron-builder
  copies it as an `extraResources` binary resolved from `process.resourcesPath`.
- Auto-updater skeleton using `electron-updater` for packaged builds; production
  signing, notarization, and release publishing are still configuration work.

## Run

Install Rust stable and Node 22+ first. On Windows, initialize the verified GNU Rust + WinLibs environment:

```powershell
.\scripts\dev-env.ps1
```

Start server:

```powershell
cargo run -p opentopia-server
```

Start desktop:

```powershell
pnpm.cmd install
pnpm.cmd dev:desktop
```

If using regular shell without PowerShell execution-policy issues:

```bash
pnpm install
pnpm dev:desktop
```

## Secure Turn Runtime

Every local API route, including health and SSE, requires a Bearer token. Electron
generates a new 256-bit token per launch; direct-server and browser development use
`OPENTOPIA_API_TOKEN` and `VITE_OPENTOPIA_API_TOKEN`. CORS accepts only packaged
file origins and configured loopback development origins.

Agent turns are serialized per thread and persisted in SQLite. `GET /api/threads/:id/turn`
reports the latest running or terminal record; `POST /api/threads/:id/turn/cancel` interrupts
its provider stream or tool future. Startup marks abandoned running/cancelling records as
interrupted. Provider SSE text is forwarded and persisted incrementally, with subscribe-before-
history replay and sequence deduplication.

Approval suspension persists the provider conversation, completed tool results,
pending calls, round number, original permission mode, and context budget. Allow
grants full access only to the exact pending call. Deny returns a structured tool
error to the model and continues that same turn.

Each provider turn receives structured recent user/assistant history after the
latest durable summary. `OPENTOPIA_CONTEXT_WINDOW_TOKENS` defaults to `128000`;
automatic LLM compaction triggers at
`OPENTOPIA_CONTEXT_COMPACT_THRESHOLD_PERCENT` (default `80`) before history is
trimmed to its bounded input budget.

## Design Boundaries

The current MVP intentionally does not yet include:

- Native Linux/macOS release-runner confinement tests and resource quotas. Windows
  strict restricted-token confinement is implemented and verified locally.
- Docker/remote sandbox execution. This is explicitly deferred for now.
- Secret values returned to renderer; only secret metadata/set/delete paths are exposed.
- Production release signing, notarization, and update publication credentials.
- Multiple named PTY sessions and shell selection; one long-lived PTY per thread is implemented.
- Product-specific GitHub/Linear/Jira/document connectors beyond the MCP host. Linear/Jira are
  explicitly deferred under the current product focus.
- PDF/Office extraction into model context and Office editing beyond the existing XLSX tool.
  In-app PDF rendering and XLSX workbook/range preview are implemented; document resources do
  not yet become model context automatically.
- Automatic re-reading of historical attachment contents. Source references remain visible
  and durable, while bounded text content applies to the Turn where the source was selected.
- Child-agent approval continuation UI and per-child token budgets. Child approval currently
  fails closed and returns control to the parent.
- Worktree desktop controls and PR/GitHub CLI.

Those are next slices; the interfaces are already shaped so they can be added without replacing the full skeleton.

## Sandbox Strategy

OpenTopia uses **OS-level local sandbox** as the execution environment security base:

- Linux: bubblewrap (filesystem/process/network namespaces; seccomp/Landlock is deferred)
- macOS: sandbox-exec with Seatbelt profiles
- Windows: Codex native sandbox helper; tested default is restricted-token/ACL
  `unelevated`, with `elevated` available for separately validated deployments

Requirements implemented for the local runtime:

- Sandbox and approval are independent controls. `read-only`, `workspace-write`,
  and `danger-full-access` define the technical boundary; the existing policy
  engine and durable approval continuation decide when execution pauses.
- `workspace-write` is the desktop default, blocks direct network access by
  default, and supports additional `writable_roots` without broad full access.
- Built-in file writes and spawned commands consume the same boundary.
- One-shot terminal commands, long-lived PTY sessions, Git actions, and MCP stdio
  processes use that same OS sandbox command plan. App/model API keys are scrubbed
  from ordinary child processes; MCP secrets require explicit `envKeys`.
- `.git`, `.agents`, and `.codex` remain protected beneath writable roots.
- `best-effort` versus `enforce` describes backend fallback only. Packaged
  execution must use `enforce`; development may use visible `best-effort` fallback.
- `danger-full-access` is explicit and never inferred from a non-interactive
  approval setting.
- An approved boundary escalation applies only to the suspended tool call. The
  continuation executes that call once with an unrestricted environment, then
  subsequent calls return to the configured sandbox.

The sandbox workbench reports the effective profile and roots. Sandbox mode,
enforcement, network, readable paths, and writable roots persist in `AppSettings` and
can be changed from Composer or Settings; environment values seed first-run defaults.
On Linux, protected metadata is
fully checked for built-in file tools and existing metadata mounts; first creation
of a missing metadata directory by a spawned command remains part of the planned
Landlock hardening.

Docker/remote sandbox is intentionally deferred for now, sharing the same `ExecutionEnvironment` trait later if resumed.

## Next Slices

Recommended order:

1. Finish provider-native image/document context and child-agent approval/token hardening.
2. Add worktree controls and PR/GitHub CLI on top of the completed desktop Git workflow.
3. Add PR/GitHub CLI and the first product-specific connectors tracked in Roadmap #4.
4. Run native Linux/macOS sandbox integration suites and add resource quotas.

Docker/Remote execution and production signing/notarization/release publication remain deferred.

## Verification

When Rust is installed:

```powershell
.\scripts\check.ps1
```

Runtime smoke test:

```powershell
.\scripts\verify-server.cmd
```

Integration smoke test:

```powershell
.\scripts\verify-integration.cmd
```
