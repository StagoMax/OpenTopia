# OpenTopia

OpenTopia is a local-first AI Coding + Work Agent MVP.

This repository currently contains:

- Rust workspace for the agent core, local server, and CLI.
- Electron + React desktop workbench.
- SQLite-backed thread and event model.
- OpenAI-compatible provider support with mock fallback.
- Built-in deterministic tools: `list_files`, `read_file`, `write_file`, `search`, `shell`, `git_diff`, `apply_patch`.
- Approval-needed flow for dangerous actions, with allow-once/deny UI.
- Electron dev shell can start the Rust server automatically when Rust is installed.
- Desktop workspace picker with recent workspaces and "open path" bridge APIs.
- Settings persistence for provider URL/model/API-key env name/permission mode.
- Workbench skeleton APIs and panels for files, read-only preview, git diff, MCP config, trajectory export, and local sandbox status.
- One long-lived PTY shell per thread with xterm.js input/output, resize, close,
  SSE replay, process-tree cleanup, and SQLite terminal history.
- Real provider-backed context summaries that are persisted and injected into later turns.
- OS sandbox adapters for Linux bubblewrap, macOS Seatbelt, and Windows Codex
  restricted-token isolation. Packaged Windows builds default to strict mode.

See `docs/source-adaptation-map.md` for the concrete source projects and modules this MVP borrows from.

## Development

Install prerequisites:

- Rust stable toolchain.
- Node.js 22+.
- pnpm 10+.

On Windows, the PowerShell scripts initialize the verified GNU Rust + WinLibs environment. If your execution policy blocks `.ps1` files, use the `.cmd` wrappers.

```powershell
.\scripts\dev-env.ps1
# or
.\scripts\dev-server.cmd
```

Start the local agent server:

```powershell
cargo run -p opentopia-server
# Windows wrapper:
.\scripts\dev-server.cmd
```

Start the desktop UI:

```powershell
pnpm install
pnpm dev:desktop
```

In the desktop UI, use **Open Workspace** in the left sidebar to pick a
directory. New threads are created with the selected `workspaceRoot`; recently
opened directories are stored in Electron user data and can be selected again
from **Recent**.

On Windows PowerShell, if `pnpm` or `npm` is blocked by execution policy, use the `.cmd` shim:

```powershell
pnpm.cmd install
pnpm.cmd dev:desktop
```

Optional provider configuration:

```powershell
$env:OPENAI_API_KEY="sk-..."
$env:OPENTOPIA_MODEL="gpt-4.1-mini"
$env:OPENTOPIA_OPENAI_BASE_URL="https://api.openai.com/v1"
cargo run -p opentopia-server -- --permission auto
```

Sandbox and approval are configured independently. The desktop defaults to a
network-restricted, workspace-write sandbox; development may explicitly fall
back when the platform helper is unavailable, while packaged builds fail closed:

```powershell
$env:OPENTOPIA_SANDBOX_MODE="workspace-write" # read-only | workspace-write | danger-full-access
$env:OPENTOPIA_SANDBOX_ENFORCEMENT="enforce"  # disabled | best-effort | enforce
$env:OPENTOPIA_SANDBOX_NETWORK="deny"
$env:OPENTOPIA_SANDBOX_WRITABLE_ROOTS="D:\shared"
```

The existing `--permission`/desktop permission control remains the approval and
tool-policy layer. Selecting a non-interactive approval mode does not disable the
sandbox; unrestricted execution requires the explicit `danger-full-access`
sandbox mode.

OpenTopia can also reuse the existing env file from the sibling credit-review project:

```powershell
$env:OPENTOPIA_ENV_FILE="J:\Project\信贷审核助手\.env"
.\scripts\dev-server.cmd
```

When `OPENTOPIA_ENV_FILE` is not set, the Windows dev scripts and Electron dev shell automatically check `J:\Project\信贷审核助手\.env`. The following aliases are supported without copying secrets:

- `CREDIT_REVIEW_LLM_API_KEY` -> `OPENTOPIA_API_KEY`
- `CREDIT_REVIEW_LLM_BASE_URL` -> `OPENTOPIA_OPENAI_BASE_URL`
- `CREDIT_REVIEW_LLM_MODEL` -> `OPENTOPIA_MODEL`

Desktop builds can also store one provider API key through Electron
`safeStorage`. The renderer process can list only metadata such as
configured status, safeStorage availability, storage backend, and the
`secrets.json` storage path under Electron `userData`; it cannot read the
secret value. When the bundled server is spawned by Electron, the main process
decrypts that key and injects it as `OPENTOPIA_API_KEY` only if an explicit
environment or `.env` value has not already configured the provider key.

Deterministic tool commands supported by the MVP:

```text
/list
/read README.md
/search AgentCore
/search crates/opentopia-core/src -- ToolResult
/write scratch/example.txt
hello from OpenTopia
/run git status --short
/diff
/patch
diff --git a/example.txt b/example.txt
new file mode 100644
--- /dev/null
+++ b/example.txt
@@ -0,0 +1 @@
+hello
```

Build a desktop installer after installing Rust:

```powershell
.\scripts\build-desktop.ps1
# or, if PowerShell scripts are blocked:
.\scripts\build-desktop.cmd
```

The desktop build script is the release packaging entry point for a clean
machine:

```powershell
pnpm.cmd install --frozen-lockfile
.\scripts\dev-env.ps1
.\scripts\build-desktop.ps1
```

It runs `cargo build --release -p opentopia-server`, stages the server binary
at `apps\desktop\resources\opentopia-server.exe` on Windows
(`opentopia-server` on Unix), then runs `electron-builder`. The desktop
`extraResources` config copies that server binary and, on Windows, the Codex
restricted-token sandbox helpers plus their Apache-2.0 license into
`process.resourcesPath`, where the packaged Electron app resolves it at
startup.

For an offline or locked-directory diagnostic build, the packaging script also
accepts `OPENTOPIA_ELECTRON_DIST` (an already extracted Electron distribution)
and `OPENTOPIA_DESKTOP_OUTPUT_DIR`. ASAR integrity and executable metadata remain
enabled unless the smoke-only `OPENTOPIA_DISABLE_ASAR_INTEGRITY=true` or
`OPENTOPIA_SKIP_EXE_EDIT=true` flags are explicitly set.

Packaged builds store SQLite under Electron `userData` rather than beside the
installed executable, so installs under `Program Files` do not require write access.

Code-signing and publish variables are intentionally placeholders until a real
release identity is available:

```powershell
# Unsigned local build:
$env:CSC_IDENTITY_AUTO_DISCOVERY="false"

# Windows signing, when available:
$env:CSC_LINK="C:\path\to\codesign.pfx"
$env:CSC_KEY_PASSWORD="..."

# GitHub draft release publishing:
$env:GH_TOKEN="..."

# macOS signing/notarization, from macOS runners:
$env:APPLE_ID="..."
$env:APPLE_APP_SPECIFIC_PASSWORD="..."
$env:APPLE_TEAM_ID="..."
```

Do not commit signing assets, tokens, or provider keys.

Run the full MVP check:

```powershell
.\scripts\check.ps1
# or, if PowerShell scripts are blocked:
.\scripts\check.cmd
```

Run the local server smoke test:

```powershell
.\scripts\verify-server.cmd
```

Run the integration smoke test:

```powershell
.\scripts\verify-integration.cmd
```

Run the real-provider context compaction smoke test (this makes one model request):

```powershell
.\scripts\verify-context-summary.cmd
```

The integration smoke test covers settings, workspace tree, search, approval
persistence, staged/unstaged hunk stage/unstage/discard, one-shot terminal
history, persistent PTY input/resize/close/history, MCP configuration,
per-thread MCP enablement, and sandbox status.

The default Windows installer output is
`apps/desktop/release/OpenTopia-0.1.0-x64.exe`. The unpacked build contains the
bundled server at `apps/desktop/release/win-unpacked/resources/opentopia-server.exe`.
