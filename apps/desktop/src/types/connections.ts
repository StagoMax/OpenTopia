export type IntegrationKind = "mcp" | "oauth_api" | "database" | "local_app";

export type IntegrationAuthScheme = "none" | "api_key" | "oauth2" | "external";

export type CapabilityDiscoveryKind = "mcp_tools_list" | "static";

export type IntegrationDefinition = {
  schemaVersion: 1;
  id: string;
  revision: number;
  key: string;
  name: string;
  description?: string | null;
  kind: IntegrationKind;
  authScheme: IntegrationAuthScheme;
  capabilityDiscovery: CapabilityDiscoveryKind;
  enabled: boolean;
  createdAt: string;
  updatedAt: string;
};

export type IntegrationDefinitionInput = {
  key: string;
  name: string;
  description?: string | null;
  kind: IntegrationKind;
  authScheme: IntegrationAuthScheme;
  capabilityDiscovery: CapabilityDiscoveryKind;
  enabled: boolean;
};

export type IntegrationDefinitionUpdate = {
  expectedRevision: number;
  name?: string;
  description?: string | null;
  clearDescription?: boolean;
  enabled?: boolean;
};

export type ConnectionOwnerType = "personal" | "org_shared" | "service_account";

export type ConnectionStatus =
  "configured" | "ready" | "degraded" | "reauth_required" | "disabled";

export type ConnectionAccount = {
  displayName?: string | null;
  externalAccountId?: string | null;
  tenantId?: string | null;
  tenantName?: string | null;
  workspaceId?: string | null;
  workspaceName?: string | null;
};

export type ConnectionAuthVerification =
  "not_required" | "unverified" | "legacy_unverified" | "verified";

export type ConnectionAuthContext = {
  credentialRef?: string | null;
  account: ConnectionAccount;
  grantedScopes: string[];
  expiresAt?: string | null;
  verification: ConnectionAuthVerification;
};

export type ConnectionAuthContextInput = {
  credentialRef?: string | null;
  account: ConnectionAccount;
  grantedScopes: string[];
  expiresAt?: string | null;
};

export type ConnectionRuntimeBinding = {
  kind: "mcp_server";
  serverId: string;
};

export type Connection = {
  schemaVersion: 1;
  id: string;
  revision: number;
  integrationDefinitionId: string;
  name: string;
  ownerType: ConnectionOwnerType;
  environment: string;
  enabled: boolean;
  status: ConnectionStatus;
  runtimeBinding: ConnectionRuntimeBinding;
  authContext: ConnectionAuthContext;
  activeCapabilityRevision?: number | null;
  lastTestedAt?: string | null;
  lastError?: string | null;
  createdAt: string;
  updatedAt: string;
};

export type ConnectionInput = {
  integrationDefinitionId: string;
  name: string;
  ownerType: ConnectionOwnerType;
  environment: string;
  enabled: boolean;
  runtimeBinding: ConnectionRuntimeBinding;
  authContext: ConnectionAuthContextInput;
};

export type ConnectionUpdate = {
  expectedRevision: number;
  name?: string;
  ownerType?: ConnectionOwnerType;
  environment?: string;
  enabled?: boolean;
  runtimeBinding?: ConnectionRuntimeBinding;
  authContext?: ConnectionAuthContextInput;
};

export type ConnectionCapability = {
  capabilityId: string;
  kind: "tool";
  name: string;
  displayName: string;
  description?: string | null;
  inputSchema: unknown;
  annotations: unknown;
  providerMetadata: {
    serverId: string;
    publicName: string;
    toolName: string;
  };
  permissionLabels: string[];
};

export type ConnectionCapabilityRevision = {
  schemaVersion: 1;
  id: string;
  connectionId: string;
  revision: number;
  source: CapabilityDiscoveryKind;
  contentHash: string;
  discoveryCoverage: {
    tools: "supported" | "unsupported";
    resources: "supported" | "unsupported";
    prompts: "supported" | "unsupported";
  };
  capabilities: ConnectionCapability[];
  discoveredAt: string;
};

export type ConnectionHealth = {
  ok: boolean;
  runtimeStatus: "not_started" | "starting" | "ready" | "error" | "disabled";
  authStatus: ConnectionAuthVerification;
  message: string;
  checkedAt: string;
  toolsCount: number;
};

export type ConnectionTestResult = {
  connection: Connection;
  health: ConnectionHealth;
};

export type ConnectionCapabilityRefreshResult = {
  connection: Connection;
  capabilityRevision: ConnectionCapabilityRevision;
  changed: boolean;
  diff: {
    addedCapabilityIds: string[];
    removedCapabilityIds: string[];
    changedCapabilityIds: string[];
  };
};
