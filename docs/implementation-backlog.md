# OpenTopia Implementation Backlog

This backlog translates `docs/ai-coding-work-agent-architecture.md` into implementation slices that can be assigned, reviewed, and verified independently.

Product priority and delivery order are tracked in GitHub
[Roadmap #4](https://github.com/StagoMax/OpenTopia/issues/4). The local-MVP implementations
for [#3 Project/Thread](https://github.com/StagoMax/OpenTopia/issues/3),
[#1 attachments/sources/Skills](https://github.com/StagoMax/OpenTopia/issues/1), and
[#2 subagents](https://github.com/StagoMax/OpenTopia/issues/2) now exist; the issues retain
their explicit hardening and multimodal follow-up work.

## Current Baseline

Implemented:

- Electron + React desktop shell.
- Rust `opentopia-core`, `opentopia-server`, and `opentopia-cli`.
- SQLite-backed threads, messages, and events.
- Bearer-authenticated local API with strict local-origin CORS and authenticated
  reconnecting fetch-SSE streams.
- SQLite-backed per-thread Turn lifecycle with running/waiting/succeeded/failed/cancelled/
  interrupted states, startup recovery, status/cancel endpoints, and a desktop Stop control.
- Incremental model/tool/event streaming with persisted token-usage events.
- OpenAI-compatible provider with env reuse from the credit review project and
  an eight-round bounded autonomous built-in/MCP tool loop.
- Built-in tools: `list_files`, `read_file`, `write_file`, `search`, `shell`, `git_diff`,
  `apply_patch`, `update_plan`, `spreadsheet`, `browser`, `spawn_agent`, `send_input`,
  `cancel_agent`, `wait_agent`, and concurrent `wait_agents`.
- Permission modes plus persistent, resumable allow-once approvals. Approval
  restores the exact provider/tool-loop state instead of starting a full-access turn.
- Dev/build scripts for Windows GNU Rust + WinLibs.
- Settings persistence, provider connectivity test, and Electron safeStorage key management.
- Workspace file tree, file preview, git diff, sandbox status, trajectory export,
  and MCP configuration skeleton APIs.
- Artifact model, SQLite artifact index, artifact read APIs, and provider-backed
  manual/automatic context compaction.
- `ExecutionEnvironment` trait and `LocalExecutionEnvironment` implementation.
- `McpExtensionHost` and `McpStdioClient` with full JSON-RPC initialize/list_tools/call_tool lifecycle.
- Per-thread terminal streaming, SQLite history, cancellation/process-tree cleanup,
  and xterm-based desktop terminal view.
- Staged/unstaged diff parsing plus validated per-hunk stage, unstage, and discard.
- Windows NSIS packaging with the Rust server bundled as an Electron resource.
- Structured recent conversation history, automatic threshold-based LLM compaction,
  and token-window trimming for every provider turn.
- SQLite-backed first-class Projects with normalized workspace-root uniqueness,
  pinned/sort metadata, project-owned Threads, reassignment/archive/delete APIs,
  recoverable unassigned/archived desktop sections, and legacy desktop migration.
- Explicit Electron attachment selection, server-side source validation/limits,
  message-persisted source and Skill references, bounded text injection, and right-rail recovery.
- Persistent subagent runs with real AgentCore execution, bounded per-parent concurrency,
  queueing, recursion limits, input/wait/cancel tools, recursive cancellation, SQLite recovery,
  SSE updates, concurrent result collection, and right-rail controls.
- Durable task plans inspired by Codex/opencode/Goose: typed plan events persist in the Thread,
  incomplete plans return to later Turns, and the desktop renders current progress.
- Local CDP Browser runtime with domain approval continuation, typed screenshots delivered as
  native provider image input, downloads, and a desktop panel that follows model tool output.
- Bounded XLSX inspect/list/read/create/update support using `calamine` and `rust_xlsxwriter`,
  routed through the same workspace policy and `ExecutionEnvironment` file boundary.
- Persisted sandbox mode/enforcement/network/read/write roots with Settings and Composer controls.
- Validated Git workflow core and sandboxed Thread API for status, branch, commit, push,
  compare, and worktree actions, with desktop status/branch/create/switch/commit/push/compare
  controls. Worktree UI and PR/GitHub CLI remain open.

## Current Product Focus

Active development is intentionally limited to:

1. End-to-end task completion and recoverable task state.
2. Built-in work tools such as spreadsheets and the browser.
3. Parent-controlled multi-agent decomposition, parallel execution, result collection,
   and synthesis.

Deferred from this focus: child-agent approval/budget hardening, Linux/macOS native sandbox
validation, Linear/Jira product connectors, top-level menu/navigation/help, Docker/Remote,
and release signing/publishing.

## P0: Make The MVP Product-Shaped

### Workspace Picker And Recents

Goal: users can open a real workspace from the desktop UI without starting from a terminal.

Status: complete for the local MVP. Electron owns directory selection and recent
workspace persistence; the renderer uses the context-isolated preload bridge.

Implemented:

- Add Electron dialog bridge for selecting directories.
- Store recent workspaces locally.
- Let new threads use the selected `workspaceRoot`.
- Add open-path action for files/directories.
- Normalize Windows paths.

Future refinement:

- Add the WSL path adapter tracked in Roadmap #4.

Acceptance:

- User can pick a folder, create a thread in it, run `/list`, and see files from that folder.
- Recent workspace survives app restart.
- Renderer still uses `contextIsolation`; no Node integration.

### Provider And Settings Persistence

Goal: model, provider, base URL, permission mode, and workspace preferences are app settings, not only env variables.

Status: MVP implementation complete. Provider validation and desktop key storage are wired into the settings UI; the renderer never receives secret values.

Implemented:

- Add settings store in SQLite or app config.
- Add server routes for reading/updating settings.
- Add desktop settings UI controls.
- Keep secrets out of localStorage; support env aliases and Electron safeStorage.
- Add provider health and explicit connectivity checks.
- Expose only key-source/keyring metadata plus set/delete operations to the renderer.

Future refinement:

- Add workspace preference controls beyond Electron recents.
- Support separate safeStorage keys per provider profile instead of one active provider key.
- Bound and incrementally summarize compacted tool history for strict compatible
  gateways; the current fallback is correct but can produce high cumulative token cost.
- Replace ad hoc completion prompting with an explicit turn-state protocol and provider-native
  compaction while preserving unrestricted long-running execution.

Acceptance:

- User can change model/base URL/permission mode from UI.
- New turns use updated settings.
- API keys are never printed in logs or returned to renderer.

### Persistent Approvals And Policy

Goal: approvals are auditable and recoverable.

Status: complete for the local MVP. Approval records and exact suspended-turn
continuations are stored in SQLite and resumed through the decision route.

Implemented:

- Add `approvals` persistence.
- Store pending/approved/denied status.
- Record action, reason, timestamps, and decision.
- Add query route for pending approvals.

Future refinement:

- Add richer exec-prefix amendments and product controls for network policy.

Acceptance:

- Pending approval survives server restart.
- Allow/deny updates stored state.
- Existing frontend approval flow keeps working.

### Search Tool

Goal: repo search is a first-class built-in tool.

Status: complete for the local MVP built-in tool and deterministic command. The
disabled app-wide sidebar search is a separate desktop feature tracked in Roadmap #4.

Implemented:

- Add `search` tool backed by `rg`.
- Add deterministic `/search` command.
- Enforce workspace read policy.
- Add output truncation metadata.

Acceptance:

- `/search AgentCore` returns matching files/lines.
- Searching outside workspace is denied or asks approval.

### Project, Sources, Skills, And Subagents

Status: local MVP complete for Roadmap #3/#1/#2.

Implemented:

- First-class Project CRUD, canonical workspace deduplication, Thread ownership/archive,
  desktop migration, rename/pin/remove, and project-scoped new tasks.
- Message parts for source and Skill references. Source contents are not copied to SQLite;
  selected canonical paths and metadata persist with the message.
- Electron context-isolated multi-file picker plus independent server validation: 20 files,
  25 MiB each, sensitive-file denial, type allowlist, canonical dedupe, 256 KiB text/file,
  and 512 KiB aggregate text injection limits.
- Codex-compatible `SKILL.md` discovery under workspace/user skill roots, YAML metadata,
  allowlisted IDs, five Skills per Turn, and 128 KiB aggregate instruction limit.
- Subagent scheduler and real AgentCore executor with inherited workspace/provider/policy/sandbox,
  four concurrent children per parent, depth two, no scheduler execution deadline, typed events, SQLite
  persistence/restart recovery, model-callable tools, HTTP controls, and desktop status/actions.

Future refinement:

- Provider-native image parts are implemented. Add PDF/Office content extraction; document
  resources currently contribute verified metadata/URI rather than parsed document text.
- Text source content is bounded and injected into the Turn where it is selected. Historical
  messages retain the source reference/metadata, but do not silently re-read a changed file
  into every later Turn.
- Add explicit missing/deleted source status refresh and richer source detail/gallery views.
- Add a child-approval UI/continuation protocol. Child approval currently fails closed and tells
  the parent to perform the action directly, so it cannot bypass the parent policy.
- Add per-subagent token accounting/budgets and configurable scheduler limits in Settings.

## P1: Coding Workbench Depth

### File Tree, Diff Review, Editor

Goal: move from raw tool output to a real coding workbench.

Status: MVP implementation complete. The workbench distinguishes staged and unstaged changes,
supports validated per-hunk stage/unstage/explicit-confirm discard, and opens workspace files or
Thread artifacts as first-class preview tabs.

Implemented:

- Add file tree panel.
- Add changed files list.
- Add authenticated preview descriptors and binary content for canonical workspace files and
  Thread-scoped artifacts.
- Add Monaco text/code preview, Blob-backed image controls, PDF.js page rendering, and virtualized
  XLSX workbook/range preview with system-application fallback for unsupported formats.
- Add selected-file diff review and explicit-confirm tracked file revert.
- Add staged/index-aware status and diff views.
- Add hunk-level stage/unstage/discard. The server re-reads and exactly matches
  the current patch, runs `git apply --check`, and only then mutates the index/worktree.

Future refinement:

- Add richer Monaco/CodeMirror side-by-side diff rendering for large files.

Acceptance:

- User can inspect current diff visually.
- Tool output no longer has to be the only way to review changes.

### Terminal Panel

Goal: command execution has an inspectable terminal-style surface.

Status: complete for the local MVP. Per-command streaming and a persistent,
per-thread PTY are both available. The desktop xterm.js instance is connected
bidirectionally to a `portable-pty` session, including resize, ANSI/ConPTY
handshake traffic, process-tree close, SSE replay, and SQLite aggregate history.
Both terminal paths use the current OS sandbox plan and cap persisted/streamed output.

Implemented:

- Add xterm.js or terminal-like output component.
- Stream shell output events incrementally.
- Add cancel/interrupt command support.
- Preserve aggregate stdout/stderr and terminal status in SQLite across restarts.
- Clean up child process trees on cancellation and timeout.
- Add one persistent shell session per thread with raw input and resize APIs.
- Merge live PTY events with persisted terminal history without duplicate seq values.
- Normalize Windows verbatim paths before passing cwd to `cmd.exe`.
- Terminate the Windows PTY process tree with `taskkill /T /F`, then persist a
  single terminal cancellation event and aggregate output.

Future refinement:

- Add shell selection and multiple named terminal sessions per thread.

Acceptance:

- Long-running command can be observed and cancelled.
- Test output remains attached to the thread.

### Context Compaction And Artifacts

Goal: long sessions stay usable.

Status: complete for the local MVP. Real provider-backed manual and automatic
compaction share one implementation. Generated summaries are persisted and every
turn receives durable summary plus bounded structured recent history.

Implemented:

- Add artifact model with inline/path storage metadata.
- Add SQLite `artifacts` table and store list/get/insert methods.
- Add artifact list/detail APIs and include artifact metadata in trajectory export.
- Add `ContextSummary` plus `context_compacted` event payload boundary.
- Add store helper for persisting oversized tool output as an artifact.
- Add artifact gallery and quick preview links in desktop UI.
- Add context status and manual compact API/UI boundary.
- Build a bounded message/event trajectory snapshot and call the active
  OpenAI-compatible provider with a dedicated summarization prompt.
- Persist provider/model/mode/covered-seq metadata and use the latest summary in
  normal turns and approval continuations.
- Add a real-provider smoke test that reuses the configured env without exposing secrets.
- Track provider-reported input/output/total token usage from streaming responses.
- Trigger automatic compaction at a configurable context-window threshold and trim
  recent history to a bounded input budget.

Future refinement:

- Add richer artifact links in every message/timeline surface.

Acceptance:

- Large command output is truncated in chat but recoverable from artifact storage.
- Thread can continue after compaction.

## P2: Extension And Work Agent Layer

### MCP Extension Host

Goal: ecosystem tools use MCP while high-trust coding tools stay built in.

Status: MVP implementation complete. Enabled MCP schemas are registered as provider tool candidates, and provider-requested MCP calls run through the same bounded autonomous loop and policy checks as built-ins.

Implemented:

- Add MCP server config model.
- Add per-thread enable/disable.
- Implement `McpStdioClient` with full stdio process lifecycle, JSON-RPC, initialize/list_tools/call_tool, timeout handling, stderr logging.
- Implement `McpExtensionHost` with tool schema caching, routing by public name, duplicate name detection.
- Restore enabled servers on startup and serialize create/update/enable/first-use startup through
  an idempotent per-server lifecycle guard with live status reporting.
- Wire `restart`, `tools`, and `call-tool` routes into the server.
- Route MCP calls through descriptor/annotation-aware policy checks.
- Register cached MCP descriptors into `AgentCore` and expose `/mcp server__tool {json}`.
- Expose desktop create/edit/restart/delete controls plus per-Thread enablement and inline status.
- Start MCP stdio through the current OS sandbox, clear inherited environment secrets,
  allow only explicit `envKeys`, refresh `tools/list_changed`, and filter schemas by
  per-Thread enablement. Direct API calls require the owning Thread.
- Parse OpenAI-compatible tool calls and run up to eight provider tool rounds,
  including enabled MCP tools, with a hard loop limit and local-summary fallback.

Future refinement:

- Persist refreshed MCP tool schemas if cache warmup should survive server restart.

Acceptance:

- A local MCP server can be started, initialized, and listed from the API.
- Tool call appears in timeline and is permission checked.

### Work Tools

Goal: support non-code work loops.

Status: Browser and bounded XLSX workbooks are first-class built-in tools and run through the
same model-controlled tool loop, policy checks, typed results, and desktop event surfaces.
Workspace files and artifacts also have a first-class, read-only preview protocol.

Implemented:

- Sandboxed Electron `WebContentsView` browser with an authenticated loopback broker and shared
  per-Thread user/Agent session; isolated CDP runtime and BrowserPanel remain the web fallback.
- XLSX inspect/list/read/create/update with structured limits and errors.
- Text/code, image, PDF, and XLSX preview tabs with authenticated content and bounded range APIs.

Deferred tasks:

- GitHub/Linear/Jira tools.
- Document/PDF extraction and editing tools beyond the completed read-only previews.
- Scheduler/reminder tool.

Acceptance:

- User can go from issue context to local code change and draft PR.

## P3: Sandbox And Distribution

### Execution Environment Abstraction

Goal: tools are decoupled from where they execute.

Status: `ExecutionEnvironment`, local path confinement, and OS sandbox command
wrapping are implemented. Packaged Windows builds default to strict `enforce`;
development defaults to `best_effort` so missing platform tooling remains visible.

Tasks:

- ~~Introduce `ExecutionEnvironment` trait.~~ (Done)
- ~~Implement local environment wrapper.~~ (Done)
- ~~Move shell/file operations behind the trait.~~ (Done)
- ~~Add OS-level local sandbox wrapping.~~ Built-in execution and stdio spawn use platform-native wrappers:
  - Linux: bubblewrap (mount/process/network namespaces; seccomp/Landlock still pending)
  - macOS: sandbox-exec + Seatbelt profile (path allowlists + port restrictions)
  - Windows: restricted token + integrity level + ACL
- Implemented now:
  - `SandboxMode` (`read-only` / `workspace-write` / `danger-full-access`) is
    independent from `OsSandboxMode` (`disabled` / `best-effort` / `enforce`),
    which now describes only backend fallback behavior.
  - `writable_roots` extends the workspace boundary without disabling the sandbox;
    built-in file writes and spawned commands share the same profile.
  - `.git`, `.agents`, and `.codex` remain protected under writable roots.
  - Approved sandbox escalation is one-shot: only the suspended call receives an
    unrestricted execution environment; later calls return to the thread sandbox.
  - Linux/macOS command builders for `bwrap` and `sandbox-exec`.
  - Windows reuses the Apache-2.0 Codex restricted-token/ACL/job-object sandbox
    helpers through `codex sandbox`; the helper binaries and license are staged in
    Windows installers. `unelevated` is the tested default; `elevated` can be
    selected but its administrator setup lifecycle still needs release validation.
  - Strict mode fails closed when the backend is unavailable. Status reports
    `ready`, `stopped`, or `error` separately from backend availability.
  - Workspace path canonicalization rejects `..` and symlink/ancestor escapes for
    file reads and writes before OS sandboxing is considered.
  - A Windows execution test proves a strict sandbox command cannot write to a
    non-temporary path outside its workspace.
  - Sandbox mode, enforcement, network, writable roots, and read paths persist in
    `AppSettings`; Settings and Composer update the AgentCore profile immediately.
  - One-shot terminal commands, long-lived PTY sessions, Git workflow actions, and
    MCP stdio processes consume the same sandbox command builder. Git responses are
    capped at 8 MiB and terminal aggregate output at 4 MiB.
- Remaining hardening:
  - Run Linux bubblewrap and macOS Seatbelt integration suites on native release
    runners; current cross-platform builders are unit tested but only Windows has
    an end-to-end confinement test in this workspace.
  - Add optional Linux seccomp/Landlock defense in depth.
  - Linux bwrap remounts existing `.git`/`.agents`/`.codex` paths read-only, but
    preventing first creation of a missing metadata directory by arbitrary shell
    commands requires the pending Landlock layer. Built-in file tools already deny it.
- Add native CPU/memory/disk quotas. Output and timeout limits are already enforced.
- ~~Add stdio session streaming for command observability.~~ (Done at terminal API level)
- ~~Add cancel/interrupt support for running commands.~~ (Done)

Acceptance:

- Shell commands are confined to the workspace by default; network access is blocked or proxied.
- Measure and publish per-platform startup overhead; do not use a single `<10ms`
  target for bwrap, Seatbelt, and Windows native sandbox backends.
- Docker implementation can be added without rewriting tool APIs.

### Docker/Remote Sandbox (Optional Backend)

Goal: provide optional container isolation for fixed CI environments and server-side deployment.

Status: data model (`SandboxDescriptor`, `ExecutionEnvironmentKind`, `SandboxLifecycle`) is defined. No runtime logic exists. **Explicitly deferred per current product decision; do not implement until the local runtime is stable.**

Tasks:

- Add `DockerExecutionEnvironment` using `bollard` or CLI.
- Add Docker workspace mount/snapshot.
- Add port forwarding (for local preview servers).
- Add sandbox lifecycle state: start/stop/pause/resume/destroy.
- Add session-scoped secret broker for API key injection.
- Add `RemoteExecutionEnvironment` skeleton for SSH/HTTP runtime proxy.

Positioning: The current priority is OS-level local sandboxing. Docker/Remote is a future direction.

Acceptance:

- A thread can execute commands inside an isolated container.
- The same built-in tool tests pass against OS-level local, Docker, and remote environments.
- Switching between OS-level local and Docker backends requires minimal configuration (if Docker is added later).

### Packaging, Signing, Updates

Goal: users can install and update the app.

Implemented:

- Build release binary into Electron resources.
- Add crash/startup logs.
- Add electron-updater channel boundary.
- Produce a verified Windows NSIS installer and unpacked app with the bundled Rust server.

Remaining external release work:

- Supply real Windows/macOS signing identities and notarization credentials.
- Configure the real GitHub release owner/repository and publication token.
- Add CI release jobs for Windows, macOS, and Linux artifacts.

Acceptance:

- Clean machine can install and launch OpenTopia without dev toolchain.
