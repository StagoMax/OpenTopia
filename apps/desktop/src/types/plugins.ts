export type McpServerConfig = {
  serverId: string;
  name: string;
  command: string;
  args: string[];
  cwd?: string | null;
  envKeys: string[];
  timeoutMs: number;
  enabled: boolean;
  pluginId?: string;
  pluginServerName?: string;
  createdAt: string;
  updatedAt: string;
};

export type McpServerInput = {
  name: string;
  command: string;
  args?: string[];
  cwd?: string;
  envKeys?: string[];
  timeoutMs?: number;
  enabled?: boolean;
};

export type McpServerStatus = {
  serverId: string;
  name: string;
  status: "not_started" | "starting" | "ready" | "error" | "disabled";
  message: string;
  toolsCount: number;
  updatedAt: string;
};

export type McpToolDescriptor = {
  publicName: string;
  serverId: string;
  toolName: string;
  description?: string | null;
  inputSchema: unknown;
  annotations: unknown;
  permissionLabels: string[];
};

export type McpServerView = {
  server: McpServerConfig;
  status: McpServerStatus;
};

export type ThreadMcpServer = {
  threadId: string;
  serverId: string;
  enabled: boolean;
  updatedAt: string;
};

export type ThreadMcpServerView = {
  server: McpServerConfig;
  binding?: ThreadMcpServer | null;
  enabled: boolean;
};

export type McpCallResult = {
  serverId: string;
  publicName: string;
  toolName: string;
  output: string;
  content: unknown[];
  structuredContent?: unknown | null;
  isError: boolean;
  raw: unknown;
};

export type SkillScope = "workspace" | "user";

export type SkillDescriptor = {
  id: string;
  name: string;
  description: string;
  path: string;
  scope: SkillScope;
  pluginId?: string;
};

export type PluginDescriptor = {
  id: string;
  name: string;
  displayName: string;
  version: string;
  description: string;
  longDescription: string;
  author: string;
  category: string;
  path: string;
  manifestPath: string;
  scope: "workspace" | "user" | "codex";
  source: "workspace" | "user" | "codex" | "bundled";
  managed: boolean;
  trust: "standard" | "official" | "privileged" | "trusted_driver";
  defaultEnabled: boolean;
  nativeCapabilities: string[];
  skillRoot?: string;
  skillCount: number;
  mcpServerCount: number;
  supportedMcpServerCount: number;
  hasApps: boolean;
  capabilities: string[];
  brandColor?: string;
  websiteUrl?: string;
  issues: string[];
};

export type PluginView = {
  plugin: PluginDescriptor;
  skillIds: string[];
  mcpServers: McpServerView[];
  effectiveEnabled: boolean;
  compatible: boolean;
};

export type PluginControlScopeType = "global" | "workspace" | "thread";
export type PluginActivationScopeType = "global" | "workspace";

export type PluginControlScope = {
  scopeType: PluginControlScopeType;
  scopeId?: string;
};

export type PluginActivationScope = {
  scopeType: PluginActivationScopeType;
  scopeId?: string;
};

export type PluginActivationRecord = {
  pluginId: string;
  scope: PluginActivationScope;
  enabled: boolean;
  updatedAt: string;
};

export type PluginSettingsRecord = {
  pluginId: string;
  scope: PluginControlScope;
  settings: unknown;
  updatedAt: string;
};

export type PluginSecretBindingRecord = {
  pluginId: string;
  scope: PluginControlScope;
  settingKey: string;
  bindingId: string;
  metadata: unknown;
  updatedAt: string;
};

export type PluginPermissionGrantStatus = "granted" | "revoked";

export type PluginPermissionGrantRecord = {
  pluginId: string;
  scope: PluginControlScope;
  permission: string;
  constraint: unknown;
  status: PluginPermissionGrantStatus;
  grantedAt?: string;
  updatedAt: string;
};

export type PluginPermissionRequest = {
  category: string;
  value: string;
  permission: string;
};

export type PluginContribution = {
  id: string;
  pluginId: string;
  localId: string;
  kind: PluginContributionKind;
  origin: "codex_compatible" | "open_topia";
  apiVersion: string;
  requiredHostCapabilities: string[];
  permissions: PluginCapabilityPermission[];
  configurationSchema?: string | null;
  declaration: unknown;
};

export type PluginRuntimeHealthStatus =
  "unknown" | "ready" | "degraded" | "error" | "stopped";

export type PluginRuntimeHealthRecord = {
  pluginId: string;
  contributionId: string;
  status: PluginRuntimeHealthStatus;
  lastError?: string;
  lastCheckedAt: string;
  restartCount: number;
};

export type PluginControlManifest = {
  apiVersion?: string;
  hostCapabilities: string[];
  permissionRequests: PluginPermissionRequest[];
  configurationSchema?: unknown;
  secretSettingKeys: string[];
  requiredSecretSettingKeys: string[];
  contributions: PluginContribution[];
};

export type PluginDetail = {
  plugin: PluginDescriptor;
  manifest: PluginControlManifest;
  activations: PluginActivationRecord[];
  effectiveEnabled: boolean;
  contributions: PluginContribution[];
  health: PluginRuntimeHealthRecord[];
};

export type PluginActivationResponse = {
  activation: PluginActivationRecord;
  effectiveEnabled: boolean;
};

export type PluginSettingsResponse = {
  schema?: unknown;
  settings: PluginSettingsRecord;
  secretBindings: PluginSecretBindingRecord[];
};

export type PluginPermissionsResponse = {
  requests: PluginPermissionRequest[];
  grants: PluginPermissionGrantRecord[];
};

export type PluginContributionKind =
  | "skill"
  | "mcp_server"
  | "native_tool"
  | "previewer"
  | "context_loader"
  | "agent_profile"
  | "scm_connector"
  | "app";

export type PluginCapabilityPermission = {
  kind: "filesystem" | "network" | "secret" | "desktop";
  value: string;
};

export type ActivatedPluginContribution = {
  pluginName: string;
  source: PluginDescriptor["source"];
  trust: PluginDescriptor["trust"];
  contribution: PluginContribution;
};

export type CapabilityUnavailableReason =
  | "disabled"
  | "host_trust_required"
  | "conflict"
  | { missing_host_capabilities: string[] }
  | { missing_permissions: PluginCapabilityPermission[] };

export type CapabilityActivationSnapshot = {
  scope: {
    workspaceId?: string | null;
    threadId?: string | null;
  };
  active: ActivatedPluginContribution[];
  unavailable: Array<{
    contribution: ActivatedPluginContribution;
    reason: CapabilityUnavailableReason;
  }>;
  conflicts: Array<{
    key: string;
    contributionIds: string[];
  }>;
};

export type ThreadPluginCapabilities = {
  pluginId: string;
  pluginName: string;
  enabled: boolean;
  contributions: PluginContribution[];
  grantedPermissions: string[];
};

export type CapabilityProjection = {
  allowAllTools: boolean;
  tools: string[];
  allowAllSkills: boolean;
  skills: string[];
  allowAllPlugins: boolean;
  plugins: string[];
  allowAllMcpServers: boolean;
  mcpServers: string[];
  allowAllWorkspaceRoots: boolean;
  workspaceRoots: string[];
};
