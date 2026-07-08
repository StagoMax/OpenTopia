# ADR 0001: MVP Desktop Shell And Runtime

## Status

Accepted for MVP.

## Decision

OpenTopia MVP uses:

- Electron + React + TypeScript for the desktop workbench.
- Rust for agent core, local app server, storage, tools, and policy.
- SQLite for local session persistence.
- SSE for agent event streaming.

Tauri remains a viable future shell, but Electron is the default for the first workbench because it provides a consistent Chromium runtime and mature desktop UI tooling.

## Rationale

This matches the practical direction of Codex App and Goose more closely than a Tauri-first shell:

- Codex App publicly references TypeScript/Node/Electron integration with Rust app server.
- Goose uses Electron Forge + React and ships Rust binaries alongside the GUI.
- opencode proves Tauri is viable, but its stack is Solid/Tauri and is better used as a lightweight alternative reference.

## Consequences

- Electron main process must not become a privileged bypass around the Rust policy engine.
- Renderer uses `contextIsolation` and no Node integration.
- All command execution, filesystem mutation, approval, and sandbox work remain in Rust.
- A `PlatformAdapter` pattern keeps the renderer portable to web or Tauri later.
