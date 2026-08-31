import type { ExperienceMode } from "./base";
import type { LibraryProviderId } from "./platform";
import type {
  WorkflowDeliveryReceipt,
  WorkflowOutput,
  WorkflowOutputReviewPolicy,
  WorkflowTrigger,
} from "./workflowAutomation";
import type {
  CapabilityActivationSnapshot,
  CapabilityProjection,
  PluginContributionRecord,
  ThreadPluginCapabilities,
} from "./plugins";

export type DataClassification =
  "public" | "internal" | "confidential" | "restricted";

export type ExecutionResourceGrant = {
  bindingId: string;
  kind: "file" | "network" | "database";
  resource: string;
  canRead: boolean;
  canWrite: boolean;
  maxDataClassification: DataClassification;
};

export type AgentModelBinding = {
  providerId: string;
  modelId: string;
};

export type AgentModelPolicy = {
  allowAllModels: boolean;
  allowedModels: AgentModelBinding[];
};

export type AgentConnectionOperationGrant = {
  operationId: string;
};

export type AgentConnectionBinding = {
  connectionId: string;
  capabilityRevision: number;
  operationGrants: AgentConnectionOperationGrant[];
};

export type SagKnowledgeBinding = {
  namespaces: string[];
};

export type ExecutionConnectionOperation = {
  connectionId: string;
  capabilityRevision: number;
  operationId: string;
  mcpServerId: string;
  providerToolName: string;
  modelToolName: string;
  pinnedOperationFingerprint: string;
};

export type RuntimeConnectionAuthority =
  | { mode: "deny_all" }
  | { mode: "legacy_mcp" }
  | {
      mode: "structured";
      operations: ExecutionConnectionOperation[];
    };

export type AgentTemplateSpec = {
  description: string;
  instructions: string;
  capabilities: CapabilityProjection;
  /**
   * Operation-level grants pinned to an immutable Connection capability
   * revision. Older persisted templates omit this field and are represented by
   * the legacy `capabilities.mcpServers` projection instead.
   */
  connectionBindings?: AgentConnectionBinding[];
  knowledgeBinding?: SagKnowledgeBinding;
  resourceGrants: ExecutionResourceGrant[];
  modelPolicy: AgentModelPolicy;
  stateSchema: unknown;
  outputSchema: unknown;
  allowAllDelegates: boolean;
  delegateTemplateIds: string[];
  budget: {
    maxTurns: number;
    maxToolCalls: number;
    maxDurationSeconds: number;
  };
  riskClass: "low" | "medium" | "high" | "critical";
};

export type AgentTemplateVersion = {
  schemaVersion: number;
  templateId: string;
  version: number;
  name: string;
  owner: string;
  spec: AgentTemplateSpec;
  status: "draft" | "published";
  contentHash: string;
  createdAt: string;
  publishedAt: string | null;
  publishedBy: string | null;
};

export type AgentCapabilityChange = {
  scope: string;
  value: string;
  kind: "added" | "removed" | "expanded" | "reduced";
};

export type AgentTemplateVersionView = {
  template: AgentTemplateVersion;
  diff: {
    fromVersion: number | null;
    toVersion: number;
    changes: AgentCapabilityChange[];
    widensCapabilities: boolean;
  };
};

export type AgentInstanceStatus =
  "active" | "suspended" | "completed" | "revoked";

export type EnterpriseExecutionContext = {
  schemaVersion: number;
  agentId: string;
  threadId: string;
  mode: ExperienceMode;
  templateId: string;
  templateVersion: number;
  parentAgentId: string | null;
  delegationChain: string[];
  capabilities: CapabilityProjection;
  connectionBindings?: AgentConnectionBinding[];
  connectionOperations?: ExecutionConnectionOperation[];
  knowledgeBinding?: SagKnowledgeBinding;
  resourceGrants: ExecutionResourceGrant[];
  modelPolicy: AgentModelPolicy;
};

export type AgentInstance = {
  schemaVersion: number;
  id: string;
  templateId: string;
  templateVersion: number;
  threadId: string;
  parentInstanceId: string | null;
  delegationDepth: number;
  executionContext: EnterpriseExecutionContext;
  state: unknown;
  stateRevision: number;
  status: AgentInstanceStatus;
  createdAt: string;
  updatedAt: string;
};

export type FlowSource =
  | { kind: "natural_language"; description: string }
  | { kind: "run_trace"; runId: string; traceHash: string };

export type FlowNodeKind =
  | "agent"
  | "skill"
  | "tool"
  | "condition"
  | "validator"
  | "approval"
  | "join"
  | "loop"
  | "output";

export type FlowGraphNode = {
  id: string;
  label: string;
  kind: FlowNodeKind;
  config: Record<string, unknown>;
  inputSchema: Record<string, unknown>;
  outputSchema: Record<string, unknown>;
};

export type FlowGraphEdge = {
  from: string;
  to: string;
  condition: string | null;
  allowedFields: string[];
  dataClassification: DataClassification;
  onError: string | null;
  loopPolicy: {
    maxIterations: number;
    continueCondition: string;
    onExhausted: "require_human" | "return_partial" | "fail";
  } | null;
};

export type FlowSpec = {
  flowId: string;
  name: string;
  description: string;
  owner: string;
  categories: string[];
  source: FlowSource;
  inputSchema: Record<string, unknown>;
  outputSchema: Record<string, unknown>;
  graph: {
    schemaVersion: number;
    entryNodeId: string;
    nodes: FlowGraphNode[];
    edges: FlowGraphEdge[];
  };
  requestedCapabilities: CapabilityProjection;
  budget: {
    maxNodeExecutions: number;
    maxToolCalls: number;
    maxDurationSeconds: number;
    maxLoopIterations: number;
  };
  riskClass: "low" | "medium" | "high" | "critical";
  pendingDecisions: string[];
};

export type FlowValidationIssue = {
  severity: "error" | "warning";
  code: string;
  message: string;
  nodeId: string | null;
  edgeIndex: number | null;
  remediation: string;
};

export type FlowValidationReport = {
  valid: boolean;
  issues: FlowValidationIssue[];
  validatedAt: string;
};

export type FlowDraft = {
  schemaVersion: number;
  id: string;
  threadId: string;
  revision: number;
  status:
    "drafting" | "reviewing" | "validating" | "ready_to_publish" | "published";
  spec: FlowSpec;
  effectiveCapabilities: CapabilityProjection;
  contentHash: string;
  lastValidation: FlowValidationReport | null;
  createdAt: string;
  updatedAt: string;
};

export type FlowTrial = {
  schemaVersion: number;
  id: string;
  draftId: string;
  draftRevision: number;
  status: "passed" | "failed";
  input: unknown;
  steps: Array<{
    order: number;
    nodeId: string;
    harnessTarget: string;
    boundedBy: number | null;
  }>;
  report: FlowValidationReport;
  createdAt: string;
};

export type FlowDraftView = {
  draft: FlowDraft;
  trials: FlowTrial[];
  testRuns: FlowRun[];
};

export type FlowDefinition = {
  schemaVersion: number;
  id: string;
  flowId: string;
  name: string;
  version: number;
  owner: string;
  description: string;
  categories: string[];
  source: FlowSource;
  graph: FlowSpec["graph"];
  inputSchema: Record<string, unknown>;
  outputSchema: Record<string, unknown>;
  capabilities: CapabilityProjection;
  budget: FlowSpec["budget"];
  riskClass: FlowSpec["riskClass"];
  contentHash: string;
  publishedAt: string;
  publishedBy: string;
};

export type WorkflowAgentSpec = {
  nodeId: string;
  templateId: string;
  templateVersion: number;
  templateContentHash: string;
  name: string;
  owner: string;
  instructions: string;
  capabilities: CapabilityProjection;
  resourceGrants: ExecutionResourceGrant[];
  modelPolicy: AgentModelPolicy;
  stateSchema: unknown;
  outputSchema: unknown;
  riskClass: "low" | "medium" | "high" | "critical";
  connectionBindings: AgentConnectionBinding[];
  knowledgeBinding?: SagKnowledgeBinding;
  connectionAuthority: RuntimeConnectionAuthority;
};

export type CompiledWorkflow = {
  schemaVersion: number;
  definitionId: string;
  flowId: string;
  flowVersion: number;
  definitionContentHash: string;
  graph: FlowSpec["graph"];
  inputSchema: unknown;
  outputSchema: unknown;
  rootCapabilities: CapabilityProjection;
  harnessCapabilities: CapabilityProjection;
  harnessConnectionAuthority: RuntimeConnectionAuthority;
  budget: FlowSpec["budget"];
  agentSpecs: Record<string, WorkflowAgentSpec>;
  contentHash: string;
};

export type FlowRevision = {
  schemaVersion: number;
  id: string;
  compiledWorkflow: CompiledWorkflow;
  trigger: WorkflowTrigger;
  ingressPolicy: "immediate" | "require_review";
  output: WorkflowOutput;
  outputReviewPolicy: WorkflowOutputReviewPolicy;
  libraryProvider?: LibraryProviderId;
  contentHash: string;
  createdAt: string;
  createdBy: string;
};

export type ActiveFlow = {
  schemaVersion: number;
  id: string;
  revision: number;
  flowId: string;
  name: string;
  threadId: string;
  status: "active" | "paused";
  activeRevision: FlowRevision;
  createdAt: string;
  updatedAt: string;
  createdBy: string;
};

export type FlowRunStatus =
  | "queued"
  | "running"
  | "pause_requested"
  | "paused"
  | "waiting_approval"
  | "waiting_human"
  | "resuming"
  | "succeeded"
  | "failed"
  | "cancel_requested"
  | "cancelled";

export type FlowNodeRun = {
  id: string;
  nodeId: string;
  attempt: number;
  status:
    | "running"
    | "waiting_approval"
    | "waiting_human"
    | "resuming"
    | "succeeded"
    | "failed"
    | "cancelled";
  input: unknown;
  output: unknown | null;
  error: string | null;
  toolCalls: number;
  transcript: FlowTranscriptEntry[];
  startedAt: string;
  completedAt: string | null;
};

export type FlowTranscriptEntry = {
  id: string;
  kind: "input" | "tool_call" | "tool_result" | "output" | "approval" | "error";
  title: string;
  content: unknown;
  toolName?: string | null;
  callId?: string | null;
  isError: boolean;
  createdAt: string;
};

export type WorkflowCheckpointStatus =
  "running" | "committed" | "failed" | "cancelled";

export type WorkflowPendingWrite = {
  nodeId: string;
  nodeRunId: string;
  result?: {
    output: unknown;
    toolCalls: number;
    transcript: FlowTranscriptEntry[];
  } | null;
  error?: string | null;
  interrupt?: unknown | null;
  resumeCommand?: unknown | null;
  completedAt: string;
};

export type WorkflowCheckpoint = {
  id: string;
  superstep: number;
  status: WorkflowCheckpointStatus;
  nodes: Array<{
    nodeId: string;
    nodeRunId: string;
    attempt: number;
    input: unknown;
  }>;
  pendingWrites: WorkflowPendingWrite[];
  createdAt: string;
  completedAt?: string | null;
};

export type WorkflowCheckpointSummary = {
  id: string;
  superstep: number;
  status: WorkflowCheckpointStatus;
  nodeIds: string[];
  pendingWriteCount: number;
  createdAt: string;
  completedAt: string;
};

export type FlowRun = {
  schemaVersion: number;
  id: string;
  threadId: string;
  flowId: string;
  flowVersion: number;
  definitionId: string;
  definitionContentHash: string;
  flowRevisionId?: string | null;
  flowRevision?: FlowRevision | null;
  testDraftId?: string | null;
  testDraftRevision?: number | null;
  revision: number;
  status: FlowRunStatus;
  input: unknown;
  ingressTrigger?: WorkflowTrigger | null;
  output: unknown | null;
  graph: FlowSpec["graph"];
  effectiveCapabilities: CapabilityProjection;
  connectionAuthority?: RuntimeConnectionAuthority;
  budget: FlowSpec["budget"];
  readyNodes: string[];
  nodeRuns: FlowNodeRun[];
  nodeOutputs: Record<string, unknown>;
  state: Record<string, unknown>;
  superstep: number;
  activeCheckpoint?: WorkflowCheckpoint | null;
  checkpointHistory: WorkflowCheckpointSummary[];
  loopCounts: Record<string, number>;
  nodeExecutions: number;
  toolCalls: number;
  outputReviewRequired: boolean;
  outputReviewed: boolean;
  waitingNodeId: string | null;
  activeHumanTaskId?: string | null;
  error: string | null;
  createdAt: string;
  startedAt: string | null;
  completedAt: string | null;
  updatedAt: string;
};

export type HumanTaskAction =
  | "approve"
  | "reject"
  | "retry"
  | "resume"
  | "submit"
  | "reconnect"
  | "acknowledge"
  | "cancel";

export type HumanTaskType =
  | "approval"
  | "input_request"
  | "output_review"
  | "recovery"
  | "reconnect"
  | "data_correction"
  | "reconciliation"
  | "manual";

export type HumanTaskStatus = "pending" | "completed" | "cancelled";

export type HumanTask = {
  schemaVersion: number;
  id: string;
  revision: number;
  threadId: string;
  sourceKind: "flow_run" | "delivery_receipt";
  sourceId: string;
  sourceNodeRunId?: string | null;
  sourceNodeId?: string | null;
  taskType: HumanTaskType;
  status: HumanTaskStatus;
  title: string;
  description: string;
  allowedActions: HumanTaskAction[];
  payload: unknown;
  actionSchema?: unknown | null;
  assignedTo?: string | null;
  claimedBy?: string | null;
  claimedAt?: string | null;
  dueAt?: string | null;
  checkpointId?: string | null;
  continuationId?: string | null;
  resolution?: {
    action: HumanTaskAction;
    note?: string | null;
    resolvedBy: string;
    resolvedAt: string;
    commandId?: string | null;
    idempotencyKey?: string | null;
    response?: unknown | null;
  } | null;
  createdAt: string;
  updatedAt: string;
  resolvedAt?: string | null;
};

export type HumanTaskResolutionResult = {
  task: HumanTask;
  run?: FlowRun | null;
  deliveryReceipt?: WorkflowDeliveryReceipt | null;
};

export type ThreadCapabilities = {
  threadId: string;
  experienceMode: ExperienceMode;
  promptProfileId: string;
  capabilityProjection: CapabilityProjection;
  workspaceRoot: string;
  generatedAt: string;
  snapshot: CapabilityActivationSnapshot;
  plugins: ThreadPluginCapabilities[];
};

export type MediaHandlerDescriptor = {
  contributionId: string;
  pluginId: string;
  localId: string;
  kind: "previewer" | "context_loader";
  extensions: string[];
  mediaTypes: string[];
  priority: number;
  runtime: string;
};

export type MediaHandlerSelection =
  | { status: "none" }
  | { status: "selected"; handler: MediaHandlerDescriptor }
  | { status: "conflict"; contributionIds: string[] };

export type MediaHandlerOperation = "preview" | "load_context";

export type MediaHandlerRuntime =
  | { type: "mcp_v1"; server: string; tool: string }
  | { type: "builtin"; adapter: string };

export type MediaHandlerResult = {
  apiVersion: "opentopia.mediaHandlerResult.v1" | string;
  kind: MediaHandlerOperation;
  payload: unknown;
};

export type MediaHandlerInvocationResponse = {
  contributionId: string;
  pluginId: string;
  runtime: MediaHandlerRuntime;
  bytesRead: number;
  output: MediaHandlerResult;
};

export type AppViewDescriptor = {
  contributionId: string;
  pluginId: string;
  localId: string;
  title: string;
  entry: string;
  allowedChannels: string[];
  sandbox: {
    nodeIntegration: false;
    allowPopups: false;
    allowTopNavigation: false;
    allowedHostApis: string[];
  };
};

export type AppViewSession = {
  sessionId: string;
  threadId: string;
  descriptor: AppViewDescriptor;
  status: "ready" | "stopped";
  startedAt: string;
  stoppedAt?: string | null;
};

export type AppViewSessionResponse = AppViewSession & {
  contentPath: string;
};

export type AppViewMessage = {
  sessionId: string;
  channel: string;
  payload: unknown;
  sentAt: string;
};

export type PluginAgentProfile = {
  name: string;
  description: string;
  developer_instructions: string;
  nickname_candidates: string[];
  model?: string | null;
  model_reasoning_effort?: string | null;
  sandbox_mode?: "read-only" | "workspace-write" | "danger-full-access" | null;
  allowed_tools?: string[] | null;
  denied_tools: string[];
  source_plugin_id?: string | null;
  source_contribution_id?: string | null;
};

export type ContributionHostSnapshot = {
  previewers: MediaHandlerDescriptor[];
  contextLoaders: MediaHandlerDescriptor[];
  apps: AppViewDescriptor[];
  agentProfiles: PluginAgentProfile[];
  issues: string[];
};
