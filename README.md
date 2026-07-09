# OpenTopia

OpenTopia is a local-first AI Coding + Work Agent MVP.

This repository currently contains:

- Rust workspace for the agent core, local server, and CLI.
- Electron + React desktop workbench.
- SQLite-backed thread and event model.
- OpenAI-compatible provider support with mock fallback.
- Built-in deterministic tools: `list_files`, `read_file`, `write_file`, `shell`, `git_diff`, `apply_patch`.
- Approval-needed flow for dangerous actions, with allow-once/deny UI.
- Electron dev shell can start the Rust server automatically when Rust is installed.

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

Deterministic tool commands supported by the MVP:

```text
/list
/read README.md
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
