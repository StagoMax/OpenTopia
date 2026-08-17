# OpenTopia Executive Summary

> **Document Status:** Final  
> **Version:** 1.0  
> **Date:** 2026-07-26  
> **Scope:** OpenTopia Desktop, Server, Agent Runtime, Built-in Tools, MCP, Subagents

---

## 1. Project Overview

OpenTopia is a **local-first AI Coding + Work Agent** desktop application. It is designed as an open-source alternative to products like OpenAI Codex, Trae Work, and Goose — providing a fully local, privacy-preserving AI agent platform that can autonomously navigate and operate within a developer's workspace.

The central philosophy is **control, observability, and recoverability**: every tool call, permission decision, file change, and agent event is recorded in a local SQLite store, displayed to the user in real time, and recoverable across process restarts.

---

## 2. Architecture

OpenTopia follows a **layered, process-separated architecture** with four major boundaries:

| Layer | Technology | Responsibility |
|---|---|---|
| Desktop Shell | Electron (main process) | Window management, local server lifecycle, secure secret storage, file dialogs, desktop browser broker |
| Renderer | React + TypeScript + Vite | Project/thread workbench, message and event display, approvals, terminal, previews, settings |
| Local API Server | Rust (Axum) | Authenticated REST/SSE endpoints, turn scheduling, terminal/PTY management, persistence orchestration |
| Agent Core | Rust core crate | Model-tool loop, policy decisions, execution environment, MCP host, browser runtime, subagent management |
| Local State | SQLite (WAL mode) | Projects, threads, messages, events, turns, approvals, artifacts, MCP config, terminal history, app settings |

### Data Flow

`
User Input
  -> UI sends request to local API Server
  -> Server persists user message, creates a Turn
  -> Agent Core requests model reasoning
  -> Model proposes tool calls (filesystem, shell, apply_patch, etc.)
  -> Policy + Sandbox check permissions
  -> Tools execute, results returned to model
  -> Model produces final response
  -> All events persisted to SQLite and streamed to UI in real time
`

---

## 3. Key Components

### 3.1 Rust Workspace (3 crates)

| Crate | Path | Role |
|---|---|---|
| opentopia-core | crates/opentopia-core/ | Domain model, agent loop, provider adapter, built-in tools, MCP client/host, policy, sandbox, execution environment, context management |
| opentopia-server | crates/opentopia-server/ | Axum HTTP server with 40+ REST/SSE endpoints, PTY management, turn lifecycle |
| opentopia-cli | crates/opentopia-cli/ | CLI frontend sharing core model and SQLite sessions |

### 3.2 Agent Loop

The agent loop follows a structured turn lifecycle:

1. Emits turn_started
2. Emits a small model delta
3. Executes deterministic local tool commands for slash commands (/list, /read, /write, /run, /diff, /patch, /mcp)
4. Calls the configured OpenAI-compatible provider (with mock fallback)
5. Parses provider tool_calls and executes built-in or MCP tools through policy checks
6. Returns tool results to provider until a terminal response
7. Emits tool start/finish events, auto-compacts older history near context-window boundary
8. Enforces a hard 270-round ceiling, reviews progress after 90-round segments
9. Emits assistant message and turn_finished

### 3.3 Built-in Tools

| Tool | Category |
|---|---|
| filesystem | Structured filesystem operations, including bounded filename find |
| shell | Terminal execution |
| shell, apply_patch | Git inspection and patch operations |
| update_plan | Task planning |
| spreadsheet | XLSX inspect/list/read/create/update |
| browser | CDP-based browser control |
| spawn_agent, send_input, cancel_agent, wait_agent, wait_agents | Multi-agent coordination |

### 3.4 MCP Extension Host

- Full stdio MCP client lifecycle (spawn, initialize, list_tools, call_tool, shutdown)
- JSON-RPC message parsing with timeout handling and stderr logging
- Tool schema caching with public-name routing and duplicate detection
- Descriptor/annotation-aware MCP policy checks (read, write, network, secret, destructive, unknown)
- Codex-compatible plugin discovery from project, user, and cache roots
- Plugin Skills namespaced as plugin-name:Skill

### 3.5 Multi-Agent Architecture

OpenTopia adopts the **Codex-style model-driven orchestration** model:

- The parent model decides whether to spawn sub-agents, task granularity, communication, and timing
- The system enforces identity uniqueness, max thread count, max depth, cross-root isolation, permission inheritance, and state transitions
- Communication is a first-class tool: agents can message each other directly via UUID or /root/... canonical paths
- Plans are persistent task memory, not a system-controlled DAG

### 3.6 Permission and Sandbox

- **Three permission modes**: Allow, Ask, Deny
- **Configurable command rules** with dangerous command detection and network policy
- **OS-level sandbox** across three platforms: Windows (restricted token/ACL/job object), Linux (bubblewrap), macOS (Seatbelt)
- **Policy layers**: user settings -> workspace rules -> AGENTS.md -> runtime decisions

### 3.7 Context Management

- Structured context assembly: fixed core -> conditional runtime modules -> experience mode -> AGENTS.md -> permission policy -> plugin/skill catalog -> world state -> conversation history -> user message
- Context compaction: dual-path design supporting both provider-native compaction (opaque state) and OpenTopia durable checkpoint with limited recent conversation tail
- Token budget enforcement with automatic threshold-based LLM compaction and window trimming

### 3.8 Terminal System

- One long-lived PTY shell per thread
- xterm.js I/O with resize, close, SSE replay
- Process-tree cleanup on cancel
- Full SQLite terminal history

---

## 4. Current Implementation Status (MVP)

### 4.1 Completed

- **Electron + React desktop shell** with project/thread workbench, approval dialogs, terminal view, preview host, settings panel, file tree, git diff panel, browser panel, artifact gallery, and right-context rail
- **Rust core**: domain model, SQLite session store, artifact model/index, built-in tool abstraction with 7+ tools, OpenAI-compatible provider + mock fallback, MCP stdio client and extension host, execution environment trait, 3-platform sandbox adapters, context summary/compaction, AGENTS.md parser, plugin/system skill integration
- **Rust server**: 40+ REST/SSE endpoints for settings, provider, projects, threads, messages, events (SSE streaming), turns, subagents, workspace, sandbox, trajectory, artifacts, previews, context, git workflow
- **Settings persistence** with Electron safeStorage key management
- **Windows NSIS packaging** with bundled Rust server
- **Permission modes** with persistent, resumable allow-once approvals
- **Multi-agent**: persistent subagent runs, bounded concurrency, recursion limits, SQLite recovery, SSE updates
- **Spreadsheet tool**: bounded XLSX support using calamine + rust_xlsxwriter
- **CDP Browser runtime**: domain approval, typed screenshots, downloads, desktop panel
- **Git workflow core**: status, branch, commit, push, compare with sandbox-backed API
- **Task plans**: typed plan events persisted in Thread with desktop progress rendering

### 4.2 In Progress / Planned

- Attachments, sources, and Skills context (Issue #1) — multimodal, approval, and resource-budget follow-up
- Subagent runtime and persistence (Issue #2) — deep hardening
- Durable checkpoint protocol for context compaction
- Provider-native compaction integration
- Worktree/PR Git operations
- Linux bubblewrap / macOS Seatbelt native verification
- Docker / Remote sandbox
- Formal release pipeline (signing, notarization, auto-update)

---

## 5. Technology Stack

| Layer | Technologies |
|---|---|
| Core Language | Rust (edition 2021, tokio async runtime) |
| Server | Axum 0.7 REST + SSE |
| Persistence | SQLite via rusqlite (bundled), WAL mode |
| Desktop Shell | Electron |
| UI | React 18, TypeScript, Vite, Tailwind CSS, Radix UI |
| Editor | Monaco Editor |
| Terminal | xterm.js + portable-pty |
| Serialization | serde / serde_json / serde_yaml |
| MCP | Custom JSON-RPC host/client |
| Browser | CDP (Chrome DevTools Protocol) |
| Spreadsheet | calamine (read), rust_xlsxwriter (write) |
| PDF | pdfjs-dist (preview) |
| AI Providers | OpenAI-compatible API, mock fallback |
| Packaging | NSIS (Windows), electron-builder |

---

## 6. Project Structure

`
OpenTopia/
  apps/desktop/           # Electron + React frontend
    electron/             # Main process, preload, updater
    src/                  # React components, API client, styles
    resources/            # Bundled server binary, sandbox helpers
  crates/
    opentopia-core/       # Agent core, domain model, tools, MCP, policy, sandbox
    opentopia-server/     # Axum REST/SSE server
    opentopia-cli/        # CLI frontend
  docs/                   # Architecture, evaluation, design documents
  evaluation/             # Evaluation harness and test suites
  scripts/                # Dev, build, verify, and packaging scripts
  seed/                   # Seed data
`

---

## 7. Design Principles

1. **Focus Separation**: UI, server, core, and desktop shell are independent processes with clear boundaries
2. **Permissions from least to most**: renderer starts with zero permissions, preload exposes specific actions, main process decides execution
3. **Controllable**: dangerous actions can be intercepted (Allow / Ask / Deny)
4. **Observable**: users see everything happening in real time
5. **Recoverable**: refresh or restart does not lose completed work
6. **Local-first**: all data stays on the user's machine by default
7. **Extensible**: MCP protocol for ecosystem tools, plugin system for skills
8. **Borrow, don't copy**: OpenTopia adapts patterns from Codex, Goose, opencode, OpenHands, and Trae without duplicating their UI pixels or complete protocols

---

## 8. Source Material and Acknowledgments

OpenTopia design and implementation draw inspiration from several open-source and commercial projects, as detailed in docs/source-adaptation-map.md. Key influences include:

- **Codex** (OpenAI): protocol/event model, approvals, exec policy, apply_patch, sandbox adapters, multi-agent orchestration, skill loader, git-utils, context runtime
- **Goose** (Block): agent loop, permission inspector, extension/MCP management, provider abstraction, Electron shell
- **opencode** (Sentry): message/tool call model, preview runtime
- **OpenHands** (All Hands AI): multi-sandbox control plane
- **Trae** (ByteDance): clean agent loop design, benchmark methodology

One explicit binary reuse: Windows sandbox helpers from Codex, redistributed under Apache-2.0 license.

---

## 9. Roadmap and Product Priority

| Priority | Feature | Status |
|---|---|---|
| P0 | Project/Thread model (Issue #3) | MVP Complete |
| P1 | Attachments, Sources, Skills (Issue #1) | MVP Complete, hardening ongoing |
| P2 | Subagent Runtime and Persistence (Issue #2) | MVP Complete, hardening ongoing |
| P3 | Context Compaction and Checkpoint Protocol | In Progress |
| P4 | Provider-native Compaction Integration | Planned |
| P5 | Worktree/PR Git Operations | Planned |
| P6 | Docker / Remote Sandbox | Planned |
| P7 | Formal Release Pipeline | Planned |

---

*For detailed architectural walkthroughs, see docs/architecture-detailed.md (Chinese) and docs/ai-coding-work-agent-architecture.md. For evaluation methodology, see docs/evaluation-system.md and docs/application-agent-evaluation-framework.md.*
