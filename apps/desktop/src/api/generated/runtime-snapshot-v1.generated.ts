/* eslint-disable */
// Generated from the Rust DTO schema. Run `pnpm contracts:generate`; do not edit.

export type RuntimeForkTurnsV1 =
  | RuntimeForkTurnsLabelV1
  | {
      count: number;
      [k: string]: unknown;
    };
export type RuntimeForkTurnsLabelV1 = "none" | "all";
export type RuntimeWorkspaceAssignmentV1 =
  | {
      mode: "shared_read_only";
      root: string;
      [k: string]: unknown;
    }
  | {
      mode: "shared_coordinated";
      root: string;
      [k: string]: unknown;
    }
  | {
      base_commit: string;
      branch: string;
      delivery_state: RuntimeWorkspaceDeliveryStateV1;
      mode: "isolated_worktree";
      repository_root: string;
      root: string;
      [k: string]: unknown;
    };
export type RuntimeWorkspaceDeliveryStateV1 = "pending" | "ready";
export type RuntimeWorkspaceModeV1 =
  "shared_read_only" | "shared_coordinated" | "isolated_worktree";

/**
 * The validated collaboration runtime contract stored in snapshot JSON.
 *
 * Security-relevant fields are strongly typed. Provider, plugin, and tool descriptors remain opaque because their owners validate them when consumed; keeping them here preserves the frozen snapshot without creating a reverse dependency from the collaboration domain into every contribution subsystem.
 */
export interface RuntimeSnapshotV1 {
  agentProfiles?: unknown[];
  agentRuntime?: unknown;
  agentType: string;
  allowedAgentTypes: string[];
  attachmentReferences?: unknown[];
  capabilityProjection?: unknown;
  forkTurns: RuntimeForkTurnsV1;
  gitBaseCommit?: string | null;
  permissionMode?: unknown;
  pluginContributions?: unknown[];
  provider?: unknown;
  sandbox?: unknown;
  schemaVersion: number;
  spawnPolicy: AgentSpawnPolicy;
  toolCatalog?: unknown[];
  tools?: string[];
  workspaceAssignment: RuntimeWorkspaceAssignmentV1;
  workspaceMode: RuntimeWorkspaceModeV1;
  workspaceRoot: string;
  [k: string]: unknown;
}
export interface AgentSpawnPolicy {
  allowChildSpawns: boolean;
  maxDepth: number;
  maxDirectChildren: number;
  [k: string]: unknown;
}
