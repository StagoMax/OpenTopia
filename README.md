# OpenTopia

OpenTopia is a local-first AI Coding + Work Agent MVP.

This repository currently contains:

- Rust workspace for the agent core, local server, and CLI.
- Electron + React desktop workbench.
- SQLite-backed thread and event model.
- OpenAI-compatible provider support with mock fallback.
- Built-in deterministic tools: `list_files`, `read_file`, `write_file`, `shell`, `git_diff`.

## Development

Install prerequisites:

- Rust stable toolchain.
- Node.js 22+.
- pnpm 10+.

Start the local agent server:

```powershell
cargo run -p opentopia-server
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
```
