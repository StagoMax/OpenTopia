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
- OS sandbox adapters for Linux bubblewrap, macOS Seatbelt, and OpenTopia's
  Windows dedicated-user/restricted-token dual backend. Packaged Windows builds
  default to strict mode.

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

`pnpm dev:desktop` opens an isolated **OpenTopia Dev** instance backed by the
Vite development server. Changes under `apps/desktop/src` are applied with hot
module replacement, so rebuilding or reinstalling the desktop package is not
required. The installed OpenTopia app may remain open at the same time.

Changes to `apps/desktop/electron` or the Rust crates require restarting
`pnpm dev:desktop` so the Electron main process or local server can restart;
they still do not require rebuilding the installer.
The managed development server uses `target/desktop-dev` by default so it can
run alongside other Cargo-launched OpenTopia servers.

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
# Optional Codex-style per-Turn rollout budget. Omit to leave it disabled.
$env:OPENTOPIA_ROLLOUT_TOKEN_LIMIT="100000"
$env:OPENTOPIA_ROLLOUT_OUTPUT_WEIGHT="1.0"
$env:OPENTOPIA_ROLLOUT_INPUT_WEIGHT="1.0"
cargo run -p opentopia-server -- --permission auto
```

The enterprise Flow surface is disabled by default and can only be enabled by
the server deployment environment (it is not a writable client preference):

```powershell
$env:OPENTOPIA_ENTERPRISE_ENABLED="true"
```

With the gate enabled, Flow sessions expose the Agent template control plane
in the right review rail. Template updates create a new immutable version;
publishing records owner approval and requires an explicit capability-expansion
decision. A published version can be instantiated more than once with isolated
state, while the instance bound to a Flow thread deterministically narrows the
Harness tool, Skill, plugin, MCP, workspace, resource, and model projection.

Sandbox and approval are configured independently. The desktop defaults to a
network-enabled, workspace-write sandbox; development may explicitly fall
back when the platform helper is unavailable, while packaged builds fail closed:

```powershell
$env:OPENTOPIA_SANDBOX_MODE="workspace-write" # read-only | workspace-write | danger-full-access
$env:OPENTOPIA_SANDBOX_ENFORCEMENT="enforce"  # disabled | best-effort | enforce
$env:OPENTOPIA_SANDBOX_NETWORK="deny" # deny (default) | allow | inherit
$env:OPENTOPIA_SANDBOX_WRITABLE_ROOTS="D:\shared"
$env:OPENTOPIA_WINDOWS_SANDBOX="auto" # auto | elevated | unelevated
```

Windows uses OpenTopia's first-party dual backend. `elevated` runs commands as
dedicated offline/online local users; the offline identity is blocked by
persistent WFP rules. `unelevated` is the restricted-token fallback and
intentionally rejects guarantees it cannot enforce, such as an
authoritative per-path deny-read rule or offline networking. `auto` uses the
dedicated-user backend after setup and otherwise falls back to a
`WRITE_RESTRICTED` token for network-enabled requests. After elevated setup,
forcing `unelevated` is rejected so a host-identity child cannot access the
broker's stored credentials; use `auto` or `elevated`.

Run elevated setup once from the built helper (Windows will request UAC):

```powershell
target\release\opentopia-sandbox.exe setup
```

The broker stores DPAPI-protected credentials, WFP configuration, a versioned
ACL ledger, and daily stage logs under `%LOCALAPPDATA%\OpenTopia\sandbox`.
Persistent workspace grants avoid rewriting ACLs for every tool invocation.
Detach a workspace and revoke the sandbox-user ACEs recorded for it with:

```powershell
target\release\opentopia-sandbox.exe cleanup --workspace J:\Project\example
```

All command sources use the same structured execution contract: resolved
runtime roots, explicit environment/stdin policy, filesystem and network
requirements, startup/execution/termination deadlines, Job Object process-tree
ownership, and staged failures. Tool adapters only supply compatibility
details (for example, headless Git or `PowerShell -NoProfile`); containment
does not depend on recognizing a particular tool.

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
- `AUDIT_COPILOT_LLM_API_KEY` -> `OPENTOPIA_API_KEY`
- `AUDIT_COPILOT_LLM_BASE_URL` -> `OPENTOPIA_OPENAI_BASE_URL`
- `AUDIT_COPILOT_LLM_MODEL` -> `OPENTOPIA_MODEL`

Desktop builds can also store one provider API key through Electron
`safeStorage`. The renderer process can list only metadata such as
configured status, safeStorage availability, storage backend, and the
`secrets.json` storage path under Electron `userData`; it cannot read the
secret value. When the bundled server is spawned by Electron, the main process
decrypts that key and injects it as `OPENTOPIA_API_KEY` only if an explicit
environment or `.env` value has not already configured the provider key.

Workspace actions are exposed as first-class UI and HTTP APIs rather than
special messages in the agent conversation. Slash-prefixed text is rejected at
the message API, so it cannot accidentally bypass the model/tool permission
flow. The deterministic workspace search endpoint is
`POST /api/threads/{thread_id}/workspace/search`; it uses the same sandboxed
`SearchTool` as the agent and is suitable for integration checks without a
provider request.

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

It builds `opentopia-server` and `opentopia-windows-sandbox` together, verifies
the helper protocol handshake, and atomically publishes a hash-verified runtime
bundle under `apps\desktop\.runtime-stage` before running `electron-builder`.
The desktop refuses to launch a packaged server/helper pair whose manifest,
hashes, or sandbox protocol do not match. Use `-StageOnly` to build and verify
the runtime bundle without invoking Electron packaging.

For an offline or locked-directory diagnostic build, the packaging script also
accepts `OPENTOPIA_ELECTRON_DIST` (an already extracted Electron distribution)
and `OPENTOPIA_DESKTOP_OUTPUT_DIR`. ASAR integrity and executable metadata remain
enabled unless the smoke-only `OPENTOPIA_DISABLE_ASAR_INTEGRITY=true` or
`OPENTOPIA_SKIP_EXE_EDIT=true` overrides are explicitly set.

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

Run the real-provider context compaction smoke test, followed by two local
structured-delta retention checks (this consumes provider API tokens):

```powershell
.\scripts\verify-context-summary.cmd
```

Configure the Electron development profile through `safeStorage`, probe a provider,
or run the deterministic two-phase long-horizon evaluation without printing the key:

```powershell
.\scripts\configure-provider-safe-storage.ps1 `
  -EnvFile "J:\Project\信贷审核助手\.env" `
  -UserDataDir ".opentopia\preview-user-data" `
  -Profile AUDIT_COPILOT_LLM

.\scripts\probe-openai-compatible.ps1 `
  -EnvFile "J:\Project\信贷审核助手\.env" `
  -Profile AUDIT_COPILOT_LLM `
  -ExpectedModel glm-5.2

.\scripts\evaluate-long-horizon.ps1 `
  -EnvFile "J:\Project\信贷审核助手\.env" `
  -Profile AUDIT_COPILOT_LLM `
  -ExpectedModel glm-5.2 `
  -TaskManifest scripts\fixtures\long-horizon\task.json

.\scripts\evaluate-long-horizon-suite.ps1 `
  -EnvFile "J:\Project\信贷审核助手\.env" `
  -Profile AUDIT_COPILOT_LLM `
  -ExpectedModel glm-5.2 `
  -Repetitions 1

.\scripts\evaluate-opentopia-tool-suite.ps1 `
  -EnvFile "J:\Project\信贷审核助手\.env" `
  -Profile AUDIT_COPILOT_LLM `
  -ExpectedModel <configured-model> `
  -Repetitions 1
```

Run the native Windows Computer Use fixture only from a dedicated evaluation
desktop or VM with no unrelated windows, accounts, or personal data. It launches
a real desktop application and automatically approves only `computer` tool calls:

```powershell
.\scripts\evaluate-computer-use.ps1 `
  -EnvFile "J:\path\to\.env" `
  -Profile AUDIT_COPILOT_LLM `
  -ExpectedModel glm-5.2 `
  -IsolatedDesktop
```

This is an OpenTopia-private fixture runner, not an OSWorld result. Its task
definitions are under `scripts/fixtures/computer-use/`.

The latest methodology, closure design, and three-task result are documented in
`docs/evaluations/glm-5.2-long-horizon-2026-07-16.md`. This local harness follows
SWE-bench/Terminal-Bench principles but is not an official leaderboard score.
The long-term task taxonomy, metric definitions, release gates, artifact schema,
and evaluation workflow are specified in `docs/evaluation-system.md`.

The integration smoke test covers settings, workspace tree, search, approval
persistence, staged/unstaged hunk stage/unstage/discard, one-shot terminal
history, persistent PTY input/resize/close/history, MCP configuration,
per-thread MCP enablement, and sandbox status.

The default Windows installer output is
`apps/desktop/release/OpenTopia-0.1.0-x64.exe`. The unpacked build contains the
bundled server at `apps/desktop/release/win-unpacked/resources/opentopia-server.exe`.
