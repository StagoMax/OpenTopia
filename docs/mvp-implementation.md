# OpenTopia MVP Implementation

This MVP is the first executable slice of the architecture described in `docs/ai-coding-work-agent-architecture.md`.

## What Exists

### Rust Core

Path: `crates/opentopia-core`

Implemented:

- Domain model: thread, message, message part, tool call, tool result, agent event.
- SQLite session store.
- Basic permission policy.
- Built-in tool abstraction.
- Built-in `list_files`, `read_file`, `write_file`, `shell`, and `git_diff` tools.
- OpenAI-compatible provider with mock fallback.
- Deterministic command parser for `/list`, `/read`, `/write`, `/run`, `/diff`.

The agent loop currently:

1. Emits `turn_started`.
2. Emits a small model delta.
3. Executes deterministic local tool commands when the user uses slash commands.
4. Otherwise calls the configured OpenAI-compatible provider, falling back to the mock provider.
5. Emits tool start/finish events.
6. Emits an assistant message.
7. Emits `turn_finished`.

### Rust Server

Path: `crates/opentopia-server`

Implemented:

- `GET /health`
- `GET /api/threads`
- `POST /api/threads`
- `GET /api/threads/{thread_id}/messages`
- `POST /api/threads/{thread_id}/messages`
- `GET /api/threads/{thread_id}/events`
- `GET /api/threads/{thread_id}/events/stream`

Events are persisted in SQLite and also broadcast through SSE.

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
- Offline state when the local server is unavailable.

## Run

Install Rust stable and Node 22+ first.

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

## Design Boundaries

The current MVP intentionally does not yet include:

- `apply_patch`.
- MCP extension host.
- Approval decision/resume API.
- Sandbox execution.
- Secret storage.
- Auto-updater packaging.

Those are next slices; the interfaces are already shaped so they can be added without replacing the full skeleton.

## Next Slices

Recommended order:

1. Add `apply_patch` tool.
2. Add approval decision/resume API.
3. Add MCP extension host.
4. Add Electron-side server bootstrap.
5. Add packaging and update pipeline.
6. Add Docker/remote sandbox.

## Verification

When Rust is installed:

```powershell
cargo check --workspace
```

When Node dependencies are installed:

```powershell
pnpm.cmd --filter @opentopia/desktop typecheck
pnpm.cmd --filter @opentopia/desktop build
```
