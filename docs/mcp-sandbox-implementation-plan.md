# MCP And Sandbox Implementation Plan

This plan turns the architecture notes into concrete implementation issues. It is based on the OpenTopia docs plus source-level review of Goose extension management, Goose permission inspection, Codex MCP protocol types, Codex OS-level sandboxing, and OpenHands sandbox routing.

## Direction

OpenTopia should keep high-trust coding tools built in, and use MCP for ecosystem/work tools. Sandbox support should be introduced as an execution abstraction first.

The execution environment follows a **local-first model**:
- **Primary: OS-level sandbox** — wrap tool execution with OS-native security mechanisms (bwrap+seccomp on Linux, Seatbelt on macOS, restricted tokens on Windows), following Codex and Claude Code's approach. This keeps the agent working directly in the user's workspace without Docker overhead.
- **Future: Docker/remote sandbox** — for fixed CI environments, multi-tenant deployments, or scenarios requiring full container isolation. Deferred priority; same `ExecutionEnvironment` trait.

## New Rust Modules

### `crates/opentopia-sandbox`

Initial files:

- `environment.rs`: `ExecutionEnvironment`, `ExecRequest`, `ExecResult`, `ProcessSpec`, `StdioSession`.
- `local.rs`: `LocalExecutionEnvironment` — wraps tool processes with OS-level sandbox primitives.
- `manager.rs`: `SandboxManager`, keyed by `thread_id` or workspace.
- `sandbox_os.rs`: platform-specific OS sandbox implementation (bwrap, Seatbelt, restricted token).

First implementation wraps local tools with OS-level sandbox where available.

### `crates/opentopia-extension-host`

Initial files:

- `config.rs`: `McpServerConfig`, `McpTransportConfig::Stdio`.
- `host.rs`: `McpExtensionHost`.
- `client.rs`: stdio MCP lifecycle.
- `tool_router.rs`: public tool name `server__tool` to server/tool mapping.
- `permissions.rs`: MCP annotations and OpenTopia manual tags to policy labels.

### `opentopia-core` Shared Types

Add:

- `execution.rs`: shared execution request/result types if a separate sandbox crate would create dependency cycles. Current MVP keeps this in core and includes `ExecutionEnvironment`, `LocalExecutionEnvironment`, file read/write requests, one-shot exec, and patch result types.
- `mcp.rs`: JSON-friendly `McpToolDescriptor`, `McpCallResult`, `McpServerStatus`.
- `tools.rs`: evolve from static `ToolRegistry` toward `ToolDescriptor` plus `ToolRuntime`.
- `policy.rs`: add `inspect_tool_call(call, descriptor, ctx)` for built-in and MCP calls.
- `sandbox.rs`: `SandboxDescriptor`, `ExecutionEnvironmentKind { Local, Docker, Remote }`, `SandboxLifecycle`.

### Server Routes

Add route modules:

- `crates/opentopia-server/src/routes/mcp.rs`
- `crates/opentopia-server/src/routes/sandbox.rs`

## MCP MVP Boundary

Do now:

- Stdio MCP servers only.
- Config fields: `name`, `command`, `args`, `cwd`, `env`, `envKeys`, `timeoutMs`, `enabled`.
- Start server and complete `initialize`.
- `list_tools`, cache schemas/descriptions/annotations.
- Expose public tool names as `server__tool`.
- `call_tool`, mapping MCP content/structured content/errors into OpenTopia `ToolResult`.
- Permission labels: `read`, `write`, `network`, `secret`, `destructive`.
- Map `annotations.readOnlyHint=true` to `read`.
- Default unknown MCP tools to `Ask`.
- Per-thread enable/disable.

Do later:

- HTTP/SSE transports.
- Frontend MCP.
- MCP prompts/resources.
- OAuth.
- Marketplace/registry.

## MCP API

Recommended routes:

- `GET /api/mcp/servers`
- `POST /api/mcp/servers`
- `PATCH /api/mcp/servers/:server_id`
- `DELETE /api/mcp/servers/:server_id`
- `POST /api/mcp/servers/:server_id/restart`
- `GET /api/mcp/servers/:server_id/tools`
- `GET /api/threads/:thread_id/mcp`
- `PUT /api/threads/:thread_id/mcp/:server_id`

Recommended tables:

- `mcp_servers`
- `mcp_server_tools`
- `thread_mcp_servers`
- optional `thread_mcp_tool_overrides`

## Sandbox Minimal Abstraction

Target trait shape:

```rust
#[async_trait]
pub trait ExecutionEnvironment: Send + Sync {
    fn id(&self) -> &str;
    fn kind(&self) -> ExecutionEnvironmentKind;
    fn workspace_root(&self) -> &Path;

    async fn exec(&self, req: ExecRequest, ctx: ExecutionContext) -> Result<ExecResult>;
    async fn spawn_stdio(&self, spec: ProcessSpec, ctx: ExecutionContext) -> Result<Box<dyn StdioSession>>;
    async fn read_file(&self, path: &Path) -> Result<Vec<u8>>;
    async fn write_file(&self, path: &Path, contents: &[u8]) -> Result<WriteResult>;
    async fn apply_patch(&self, patch: &str, ctx: ExecutionContext) -> Result<PatchResult>;
}
```

`spawn_stdio` belongs in the first abstraction because MCP stdio servers are long-lived bidirectional processes, not ordinary one-shot commands.

## Development Order

1. Extract `ExecutionEnvironment`.
   Status: local core boundary added; built-in read/write/run/patch/diff and primary rg search now route through `LocalExecutionEnvironment`.
   Acceptance: existing `/read`, `/write`, `/run`, `/patch` behavior remains unchanged.

2. Add `ToolDescriptor` and `ToolRuntime`.
   Acceptance: built-in tool schemas can be listed, and agent execution no longer depends only on a static registry.

3. Add MCP config store and routes.
   Acceptance: stdio server configs can be created, edited, deleted, and enabled per thread.

4. Implement stdio `McpExtensionHost`.
   Acceptance: a test MCP server can be started, initialized, and listed.

5. Route MCP tool calls through the normal tool event path.
   Acceptance: calls emit `ToolCallStarted` and `ToolCallFinished`; disabled tools return clear errors.

6. Apply permission labels to MCP tools.
   Acceptance: read-only tools can run in read/auto modes; write/network/secret/destructive or unknown tools ask in approve mode.

7. **Add OS-level local sandbox wrapping**.
   Status: local sandbox config and platform command builders are implemented in
   `opentopia-core`; `LocalExecutionEnvironment.exec()` and `spawn_stdio()` route
   through the wrapper boundary. Packaged Windows builds default to strict mode;
   development defaults to explicit best-effort mode.
   Wrap `LocalExecutionEnvironment.exec()` with platform-native sandbox:
   - Linux: bubblewrap + seccomp (file system mount namespace + network namespace isolation)
   - macOS: sandbox-exec with Seatbelt profile (read/write path allowlists + network restrictions)
   - Windows: restricted token / integrity level isolation
   Current implementation:
   - Linux builder emits a `bwrap` command with workspace write bind, extra read allowlists, and optional network namespace isolation.
   - macOS builder emits `sandbox-exec -p <profile>` with workspace read/write allowlists and network denied by default.
   - Windows invokes the packaged Codex restricted-token/ACL/job-object helper via
     `codex sandbox`. `enforce` fails closed if that backend is absent; best-effort
     reports an explicit passthrough instead of claiming isolation.
   Acceptance: shell commands are confined to workspace paths; network access is blocked or proxied.

8. Add local sandbox status route.
   Acceptance: API can report current thread environment kind, id, workspace, and status.

9. **Docker/remote sandbox (optional backend, deferred priority)**.
   Add `DockerExecutionEnvironment` as an alternative backend sharing the same `ExecutionEnvironment` trait.
   Acceptance: the same built-in tool tests pass against OS-level local, Docker, and optionally remote environments; Docker is not the default for local development.

## OS-Level Sandbox: Borrow Patterns (Not Code)

- **Codex `sandboxing/src/bwrap.rs` + `linux-sandbox/`**: borrow Linux bwrap + seccomp + Landlock wrapping pattern. OpenTopia will reimplement in its own Rust async style rather than copy Codex's synchronous C-FFI approach.
- **Codex `sandboxing/src/seatbelt/`**: borrow macOS sandbox-exec plus Seatbelt `.sbpl` profile pattern. OpenTopia will define its own profile templates.
- **Codex `sandboxing/src/windows.rs` + `windows-sandbox-rs/`**: borrow Windows restricted token + ACL + integrity level isolation pattern.
- **Claude Code `sandbox-runtime/`**: borrow the conceptual model of dual sandbox layers (outer layer = file system + network isolation, inner layer = seccomp/syscall filtering). Also borrow the read-denylist-then-allowlist and write-allowlist-then-denylist permission model.

## Borrow, But Do Not Copy

- Goose `extension_manager.rs`: borrow extension lifecycle, tool prefixing, cache invalidation, and config restart patterns. Avoid Goose-specific platform extension and global config coupling.
- Goose `permission_inspector.rs`: borrow annotation-driven read-only handling and allow/deny/ask layering. Avoid SmartApprove until deterministic policy is stronger.
- Codex `protocol/src/mcp.rs`: borrow JSON/TS/schema-friendly type modeling. Do not adopt the full Codex protocol.
- Codex `sandboxing/`: borrow OS-level sandbox architecture patterns (which platform primitive to use). Do not copy Codex's synchronous C-FFI wrapper or its exec-server protocol.
- OpenHands `sandbox_router.py`: borrow lifecycle and secret-broker direction for the optional Docker/remote backend if added later. Do not copy the Python app server or user/auth control plane into the local MVP.
- OpenHands `docker_sandbox_service.py`: borrow Docker SDK patterns (health check polling, port allocation, container naming) for the optional Docker backend if added later. Do not copy Python-specific async patterns.
