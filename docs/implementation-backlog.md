# OpenTopia Implementation Backlog

This backlog translates `docs/ai-coding-work-agent-architecture.md` into implementation slices that can be assigned, reviewed, and verified independently.

## Current Baseline

Implemented:

- Electron + React desktop shell.
- Rust `opentopia-core`, `opentopia-server`, and `opentopia-cli`.
- SQLite-backed threads, messages, and events.
- Bearer-authenticated local API with strict local-origin CORS and authenticated
  reconnecting fetch-SSE streams.
- Per-thread serialized agent turns with status/cancel endpoints and a desktop Stop control.
- Incremental model/tool/event streaming with persisted token-usage events.
- OpenAI-compatible provider with env reuse from the credit review project and
  an eight-round bounded autonomous built-in/MCP tool loop.
- Built-in tools: `list_files`, `read_file`, `write_file`, `shell`, `git_diff`, `apply_patch`.
- Permission modes plus persistent, resumable allow-once approvals. Approval
  restores the exact provider/tool-loop state instead of starting a full-access turn.
- Dev/build scripts for Windows GNU Rust + WinLibs.
- Settings persistence, provider connectivity test, and Electron safeStorage key management.
- Workspace file tree, file preview, git diff, sandbox status, trajectory export,
  and MCP configuration skeleton APIs.
- Artifact model, SQLite artifact index, artifact read APIs, and context
  compaction event/API skeleton.
- `ExecutionEnvironment` trait and `LocalExecutionEnvironment` implementation.
- `McpExtensionHost` and `McpStdioClient` with full JSON-RPC initialize/list_tools/call_tool lifecycle.
- Per-thread terminal streaming, SQLite history, cancellation/process-tree cleanup,
  and xterm-based desktop terminal view.
- Staged/unstaged diff parsing plus validated per-hunk stage, unstage, and discard.
- Windows NSIS packaging with the Rust server bundled as an Electron resource.
- Structured recent conversation history, automatic threshold-based LLM compaction,
  and token-window trimming for every provider turn.

## P0: Make The MVP Product-Shaped

### Workspace Picker And Recents

Goal: users can open a real workspace from the desktop UI without starting from a terminal.

Tasks:

- Add Electron dialog bridge for selecting directories.
- Store recent workspaces locally.
- Let new threads use the selected `workspaceRoot`.
- Add open-path action for files/directories.
- Normalize Windows paths and leave a WSL adapter seam.

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

Acceptance:

- User can change model/base URL/permission mode from UI.
- New turns use updated settings.
- API keys are never printed in logs or returned to renderer.

### Persistent Approvals And Policy

Goal: approvals are auditable and recoverable.

Tasks:

- Add `approvals` persistence.
- Store pending/approved/denied status.
- Record action, reason, timestamps, and decision.
- Add query route for pending approvals.
- Expand policy types for future exec prefix rules and network policy.

Acceptance:

- Pending approval survives server restart.
- Allow/deny updates stored state.
- Existing frontend approval flow keeps working.

### Search Tool

Goal: repo search is a first-class built-in tool.

Tasks:

- Add `search` tool backed by `rg`.
- Add deterministic `/search` command.
- Enforce workspace read policy.
- Add output truncation metadata.

Acceptance:

- `/search AgentCore` returns matching files/lines.
- Searching outside workspace is denied or asks approval.

## P1: Coding Workbench Depth

### File Tree, Diff Review, Editor

Goal: move from raw tool output to a real coding workbench.

Status: MVP implementation complete. The workbench distinguishes staged and unstaged changes and supports validated per-hunk stage, unstage, and explicit-confirm discard.

Implemented:

- Add file tree panel.
- Add changed files list.
- Add file preview/read-only editor.
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
- Wire `restart`, `tools`, and `call-tool` routes into the server.
- Route MCP calls through descriptor/annotation-aware policy checks.
- Register cached MCP descriptors into `AgentCore` and expose `/mcp server__tool {json}`.
- Parse OpenAI-compatible tool calls and run up to eight provider tool rounds,
  including enabled MCP tools, with a hard loop limit and local-summary fallback.

Future refinement:

- Persist refreshed MCP tool schemas if cache warmup should survive server restart.

Acceptance:

- A local MCP server can be started, initialized, and listed from the API.
- Tool call appears in timeline and is permission checked.

### Work Tools

Goal: support non-code work loops.

Tasks:

- GitHub/Linear/Jira tools.
- Browser automation bridge.
- Docs/Sheets/PDF MCP servers.
- Scheduler/reminder tool.
- Artifact gallery.

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
- Remaining hardening:
  - Persist sandbox mode/network/writable roots in `AppSettings` and expose a
    separate composer/settings selector. The current desktop displays effective
    sandbox state, while changes are configured through environment variables.
  - Run Linux bubblewrap and macOS Seatbelt integration suites on native release
    runners; current cross-platform builders are unit tested but only Windows has
    an end-to-end confinement test in this workspace.
  - Add optional Linux seccomp/Landlock defense in depth.
  - Linux bwrap remounts existing `.git`/`.agents`/`.codex` paths read-only, but
    preventing first creation of a missing metadata directory by arbitrary shell
    commands requires the pending Landlock layer. Built-in file tools already deny it.
- Add resource limits (CPU/memory/disk quotas).
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
