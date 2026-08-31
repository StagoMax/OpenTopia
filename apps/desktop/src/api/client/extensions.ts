import type {
  ActiveFlow,
  AgentInstance,
  AgentInstanceStatus,
  AgentModelPolicy,
  AgentTemplateSpec,
  AgentTemplateVersionView,
  AppViewMessage,
  AppViewSession,
  AppViewSessionResponse,
  CapabilityProjection,
  ContributionHostSnapshot,
  ExecutionResourceGrant,
  FlowCase,
  FlowCaseResult,
  FlowDraftView,
  FlowRun,
  FlowSpec,
  FlowTrial,
  HumanTask,
  HumanTaskAction,
  HumanTaskResolutionResult,
  HumanTaskStatus,
  HumanTaskType,
  MediaHandlerInvocationResponse,
  MediaHandlerOperation,
  MediaHandlerSelection,
  PluginActivationResponse,
  PluginContributionRecord,
  PluginControlScope,
  PluginDetail,
  PluginPermissionGrantRecord,
  PluginPermissionsResponse,
  PluginRuntimeHealthRecord,
  PluginSettingsResponse,
  PluginView,
  ThreadCapabilities,
  WorkflowDeliveryReceipt,
  WorkflowDeliveryStatus,
  WorkflowEvaluation,
  WorkflowEvaluationSummary,
  WorkflowOutput,
  WorkflowOutputReviewPolicy,
} from "../../types";
import type { AgentTemplateConnectionAccessView } from "../generated/desktop-http-v1.generated";
import { ApiResponseError, queryString } from "./transport";
import { ConfigurationApi } from "./configuration";

export class ExtensionsApi extends ConfigurationApi {
  async listPlugins(input?: {
    workspaceRoot?: string | null;
    threadId?: string | null;
  }): Promise<PluginView[]> {
    return this.get(
      "listPlugins",
      `/api/plugins${queryString({
        workspaceRoot: input?.workspaceRoot ?? undefined,
        threadId: input?.threadId ?? undefined,
      })}`,
    );
  }

  async installPlugin(path: string): Promise<PluginView> {
    return this.post("installPlugin", "/api/plugins/install", { path });
  }

  async uninstallPlugin(
    pluginId: string,
    workspaceRoot?: string | null,
  ): Promise<void> {
    await this.post("uninstallPlugin", "/api/plugins/uninstall", {
      pluginId,
      workspaceRoot: workspaceRoot ?? undefined,
    });
  }

  async setThreadPlugin(
    threadId: string,
    pluginId: string,
    enabled: boolean,
  ): Promise<PluginView> {
    return this.put("setThreadPlugin", `/api/threads/${threadId}/plugins`, {
      pluginId,
      enabled,
    });
  }

  async getPluginDetail(
    pluginId: string,
    input?: { workspaceRoot?: string | null; threadId?: string | null },
  ): Promise<PluginDetail> {
    return this.get(
      "getPluginDetail",
      `/api/plugins/${encodeURIComponent(pluginId)}${queryString({
        workspaceRoot: input?.workspaceRoot ?? undefined,
        threadId: input?.threadId ?? undefined,
      })}`,
    );
  }

  async setPluginActivation(
    pluginId: string,
    scope: PluginControlScope,
    enabled: boolean,
  ): Promise<PluginActivationResponse> {
    return this.put(
      "setPluginActivation",
      `/api/plugins/${encodeURIComponent(pluginId)}/activation`,
      {
        scope,
        enabled,
      },
    );
  }

  async getPluginSettings(
    pluginId: string,
    scope: PluginControlScope,
  ): Promise<PluginSettingsResponse> {
    return this.get(
      "getPluginSettings",
      `/api/plugins/${encodeURIComponent(pluginId)}/settings${queryString({
        scopeType: scope.scopeType,
        scopeId: scope.scopeId,
      })}`,
    );
  }

  async updatePluginSettings(
    pluginId: string,
    scope: PluginControlScope,
    settings: Record<string, unknown>,
    secretBindings: Record<string, string | null>,
  ): Promise<PluginSettingsResponse> {
    return this.patch(
      "updatePluginSettings",
      `/api/plugins/${encodeURIComponent(pluginId)}/settings`,
      {
        scope,
        settings,
        secretBindings,
      },
    );
  }

  async getPluginPermissions(
    pluginId: string,
    input?: { workspaceRoot?: string | null; threadId?: string | null },
  ): Promise<PluginPermissionsResponse> {
    return this.get(
      "getPluginPermissions",
      `/api/plugins/${encodeURIComponent(pluginId)}/permissions${queryString({
        workspaceRoot: input?.workspaceRoot ?? undefined,
        threadId: input?.threadId ?? undefined,
      })}`,
    );
  }

  async setPluginPermission(
    pluginId: string,
    input: {
      scope: PluginControlScope;
      permission: string;
      constraint: unknown;
      granted: boolean;
    },
  ): Promise<PluginPermissionGrantRecord> {
    return this.put(
      "setPluginPermission",
      `/api/plugins/${encodeURIComponent(pluginId)}/permissions`,
      input,
    );
  }

  async getPluginContributions(
    pluginId: string,
    input?: { workspaceRoot?: string | null; threadId?: string | null },
  ): Promise<PluginContributionRecord[]> {
    return this.get(
      "getPluginContributions",
      `/api/plugins/${encodeURIComponent(pluginId)}/contributions${queryString({
        workspaceRoot: input?.workspaceRoot ?? undefined,
        threadId: input?.threadId ?? undefined,
      })}`,
    );
  }

  async getPluginHealth(
    pluginId: string,
    input?: { workspaceRoot?: string | null; threadId?: string | null },
  ): Promise<PluginRuntimeHealthRecord[]> {
    return this.get(
      "getPluginHealth",
      `/api/plugins/${encodeURIComponent(pluginId)}/health${queryString({
        workspaceRoot: input?.workspaceRoot ?? undefined,
        threadId: input?.threadId ?? undefined,
      })}`,
    );
  }

  async getThreadCapabilities(threadId: string): Promise<ThreadCapabilities> {
    return this.get(
      "getThreadCapabilities",
      `/api/threads/${encodeURIComponent(threadId)}/capabilities`,
    );
  }

  async listAgentTemplates(
    includeArchived = false,
  ): Promise<AgentTemplateVersionView[]> {
    return this.get(
      "listAgentTemplates",
      `/api/agent-templates${queryString({
        includeArchived: includeArchived ? "true" : undefined,
      })}`,
    );
  }

  async getAgentTemplateConnectionAccess(
    templateId: string,
    version: number,
    signal?: AbortSignal,
  ): Promise<AgentTemplateConnectionAccessView> {
    return this.get(
      "getAgentTemplateConnectionAccess",
      `/api/agent-templates/${encodeURIComponent(templateId)}/versions/${version}/connection-access`,
      signal,
    );
  }

  async createAgentTemplateVersion(input: {
    templateId: string;
    name: string;
    owner: string;
    spec: AgentTemplateSpec;
  }): Promise<AgentTemplateVersionView> {
    return this.post(
      "createAgentTemplateVersion",
      "/api/agent-templates",
      input,
    );
  }

  async publishAgentTemplateVersion(
    templateId: string,
    version: number,
    input: {
      approvedBy: string;
      approveCapabilityExpansion: boolean;
    },
  ): Promise<AgentTemplateVersionView> {
    return this.post(
      "publishAgentTemplateVersion",
      `/api/agent-templates/${encodeURIComponent(templateId)}/versions/${version}/publish`,
      input,
    );
  }

  async deleteAgentTemplateVersion(
    templateId: string,
    version: number,
  ): Promise<void> {
    await this.delete(
      "deleteAgentTemplateVersion",
      `/api/agent-templates/${encodeURIComponent(templateId)}/versions/${version}`,
    );
  }

  async archiveAgentTemplate(templateId: string): Promise<void> {
    await this.delete(
      "archiveAgentTemplate",
      `/api/agent-templates/${encodeURIComponent(templateId)}`,
    );
  }

  async createAgentInstance(input: {
    templateId: string;
    templateVersion?: number;
    threadId: string;
    parentInstanceId?: string;
    requestedCapabilities?: CapabilityProjection;
    requestedResourceGrants?: ExecutionResourceGrant[];
    requestedModelPolicy?: AgentModelPolicy;
    initialState: unknown;
    bindToThread?: boolean;
  }): Promise<{ instance: AgentInstance; bound: boolean }> {
    return this.post("createAgentInstance", "/api/agent-instances", input);
  }

  async listAgentInstances(
    filters: {
      templateId?: string;
      status?: AgentInstanceStatus;
      limit?: number;
    } = {},
  ): Promise<AgentInstance[]> {
    return this.get(
      "listAgentInstances",
      `/api/agent-instances${queryString(filters)}`,
    );
  }

  async listThreadAgentInstances(threadId: string): Promise<AgentInstance[]> {
    return this.get(
      "listThreadAgentInstances",
      `/api/threads/${threadId}/agent-instances`,
    );
  }

  async getBoundThreadAgentInstance(
    threadId: string,
  ): Promise<AgentInstance | null> {
    return this.get(
      "getBoundThreadAgentInstance",
      `/api/threads/${threadId}/agent-instance`,
    );
  }

  async bindThreadAgentInstance(
    threadId: string,
    instanceId: string,
  ): Promise<AgentInstance> {
    return this.put(
      "bindThreadAgentInstance",
      `/api/threads/${threadId}/agent-instance/${instanceId}`,
      {},
    );
  }

  async updateAgentInstance(
    instanceId: string,
    input: {
      state?: unknown;
      expectedStateRevision?: number;
      status?: AgentInstanceStatus;
    },
  ): Promise<AgentInstance> {
    return this.patch(
      "updateAgentInstance",
      `/api/agent-instances/${instanceId}`,
      input,
    );
  }

  async listFlows(
    filters: { query?: string; status?: ActiveFlow["status"] } = {},
  ): Promise<ActiveFlow[]> {
    return this.get(
      "listFlows",
      `/api/flows${queryString({
        query: filters.query || undefined,
        status: filters.status,
      })}`,
    );
  }

  async getFlow(flowId: string): Promise<ActiveFlow> {
    return this.get("getFlow", `/api/flows/${encodeURIComponent(flowId)}`);
  }

  async invokeFlow(
    flowId: string,
    input: { idempotencyKey: string; input?: unknown },
  ): Promise<FlowCaseResult> {
    return this.post(
      "invokeFlow",
      `/api/flows/${encodeURIComponent(flowId)}/invoke`,
      input,
    );
  }

  async dispatchFlowEvent(input: {
    source: string;
    eventType: string;
    idempotencyKey: string;
    payload?: unknown;
  }): Promise<FlowCaseResult[]> {
    return this.post("dispatchFlowEvent", "/api/flow-events", input);
  }

  async listFlowCases(filters: { flowId?: string } = {}): Promise<FlowCase[]> {
    return this.get(
      "listFlowCases",
      `/api/flow-cases${queryString({ flowId: filters.flowId })}`,
    );
  }

  async startPendingFlowCase(caseId: string): Promise<FlowCaseResult> {
    return this.post(
      "startPendingFlowCase",
      `/api/flow-cases/${encodeURIComponent(caseId)}/start`,
      {},
    );
  }

  async supersedePendingFlowCase(
    caseId: string,
    input: { replacementCaseId?: string; note: string },
  ): Promise<FlowCase> {
    return this.post(
      "supersedePendingFlowCase",
      `/api/flow-cases/${encodeURIComponent(caseId)}/supersede`,
      input,
    );
  }

  async listFlowDeliveryReceipts(
    filters: {
      flowRevisionId?: string;
      status?: WorkflowDeliveryStatus;
    } = {},
  ): Promise<WorkflowDeliveryReceipt[]> {
    return this.get(
      "listFlowDeliveryReceipts",
      `/api/flow-delivery-receipts${queryString({
        flowRevisionId: filters.flowRevisionId,
        status: filters.status,
      })}`,
    );
  }

  async retryFlowDelivery(
    receiptId: string,
    expectedRevision: number,
  ): Promise<WorkflowDeliveryReceipt> {
    return this.post(
      "retryFlowDelivery",
      `/api/flow-delivery-receipts/${encodeURIComponent(receiptId)}/retry`,
      { expectedRevision },
    );
  }

  async listFlowEvaluations(
    filters: { flowRevisionId?: string } = {},
  ): Promise<WorkflowEvaluation[]> {
    return this.get(
      "listFlowEvaluations",
      `/api/flow-evaluations${queryString({ flowRevisionId: filters.flowRevisionId })}`,
    );
  }

  async createFlowEvaluation(input: {
    runId: string;
    evaluator: string;
    score: number;
    passed: boolean;
    labels?: string[];
    note?: string;
  }): Promise<WorkflowEvaluation> {
    return this.post("createFlowEvaluation", "/api/flow-evaluations", input);
  }

  async getFlowEvaluationSummary(
    flowRevisionId: string,
  ): Promise<WorkflowEvaluationSummary> {
    return this.get(
      "getFlowEvaluationSummary",
      `/api/flow-evaluation-summary${queryString({ flowRevisionId })}`,
    );
  }

  async pauseFlow(
    flowId: string,
    expectedRevision: number,
  ): Promise<ActiveFlow> {
    return this.post(
      "pauseFlow",
      `/api/flows/${encodeURIComponent(flowId)}/pause`,
      { expectedRevision },
    );
  }

  async resumeFlow(
    flowId: string,
    expectedRevision: number,
  ): Promise<ActiveFlow> {
    return this.post(
      "resumeFlow",
      `/api/flows/${encodeURIComponent(flowId)}/resume`,
      { expectedRevision },
    );
  }

  async copyFlow(
    flowId: string,
    input: { flowId: string; name: string; owner: string },
  ): Promise<FlowDraftView> {
    return this.post(
      "copyFlow",
      `/api/flows/${encodeURIComponent(flowId)}/copy`,
      input,
    );
  }

  async listFlowDrafts(threadId: string): Promise<FlowDraftView[]> {
    return this.get(
      "listFlowDrafts",
      `/api/threads/${encodeURIComponent(threadId)}/flow-drafts`,
    );
  }

  async getThreadFlowDraft(threadId: string): Promise<FlowDraftView | null> {
    return this.get(
      "getThreadFlowDraft",
      `/api/threads/${encodeURIComponent(threadId)}/flow-draft`,
    );
  }

  async createFlowDraft(
    threadId: string,
    spec: FlowSpec,
  ): Promise<FlowDraftView> {
    return this.post(
      "createFlowDraft",
      `/api/threads/${encodeURIComponent(threadId)}/flow-drafts`,
      { spec },
    );
  }

  async updateFlowDraft(
    draftId: string,
    expectedRevision: number,
    spec: FlowSpec,
  ): Promise<FlowDraftView> {
    return this.put(
      "updateFlowDraft",
      `/api/flow-drafts/${encodeURIComponent(draftId)}`,
      {
        expectedRevision,
        spec,
      },
    );
  }

  async validateFlowDraft(draftId: string): Promise<FlowDraftView> {
    return this.post(
      "validateFlowDraft",
      `/api/flow-drafts/${encodeURIComponent(draftId)}/validate`,
      {},
    );
  }

  async simulateFlowDraft(
    draftId: string,
    input: unknown = {},
  ): Promise<FlowTrial> {
    return this.post(
      "simulateFlowDraft",
      `/api/flow-drafts/${encodeURIComponent(draftId)}/simulate`,
      { input },
    );
  }

  async startFlowTestRun(
    draftId: string,
    input: unknown = {},
    startedBy = "local-user",
  ): Promise<FlowRun> {
    return this.post(
      "startFlowTestRun",
      `/api/flow-drafts/${encodeURIComponent(draftId)}/test-run`,
      { input, startedBy },
    );
  }

  async activateFlowDraft(
    draftId: string,
    input: {
      activatedBy: string;
      expectedFlowRevision?: number;
      output?: WorkflowOutput;
      outputReviewPolicy?: WorkflowOutputReviewPolicy;
    },
  ): Promise<ActiveFlow> {
    return this.post(
      "activateFlowDraft",
      `/api/flow-drafts/${encodeURIComponent(draftId)}/activate`,
      input,
    );
  }

  async listFlowRuns(threadId: string): Promise<FlowRun[]> {
    return this.get(
      "listFlowRuns",
      `/api/threads/${encodeURIComponent(threadId)}/flow-runs`,
    );
  }

  async listAllFlowRuns(
    filters: { status?: FlowRun["status"]; limit?: number } = {},
  ): Promise<FlowRun[]> {
    return this.get("listAllFlowRuns", `/api/flow-runs${queryString(filters)}`);
  }

  async getFlowRun(runId: string): Promise<FlowRun> {
    return this.get(
      "getFlowRun",
      `/api/flow-runs/${encodeURIComponent(runId)}`,
    );
  }

  async pauseFlowRun(runId: string): Promise<FlowRun> {
    return this.post(
      "pauseFlowRun",
      `/api/flow-runs/${encodeURIComponent(runId)}/pause`,
      {},
    );
  }

  async resumeFlowRun(
    runId: string,
    input: {
      approved?: boolean;
      note?: string;
      retryInterruptedNode?: boolean;
    } = {},
  ): Promise<FlowRun> {
    return this.post(
      "resumeFlowRun",
      `/api/flow-runs/${encodeURIComponent(runId)}/resume`,
      input,
    );
  }

  async cancelFlowRun(runId: string): Promise<FlowRun> {
    return this.post(
      "cancelFlowRun",
      `/api/flow-runs/${encodeURIComponent(runId)}/cancel`,
      {},
    );
  }

  async listHumanTasks(
    filters: {
      status?: HumanTaskStatus;
      kind?: HumanTaskType;
      threadId?: string;
      flowRunId?: string;
    } = {},
    signal?: AbortSignal,
  ): Promise<HumanTask[]> {
    return this.get(
      "listHumanTasks",
      `/api/human-tasks${queryString({
        status: filters.status ?? "pending",
        kind: filters.kind,
        threadId: filters.threadId,
        flowRunId: filters.flowRunId,
      })}`,
      signal,
    );
  }

  async getHumanTask(taskId: string, signal?: AbortSignal): Promise<HumanTask> {
    return this.get(
      "getHumanTask",
      `/api/human-tasks/${encodeURIComponent(taskId)}`,
      signal,
    );
  }

  async resolveHumanTask(
    taskId: string,
    input: {
      expectedRevision: number;
      action: HumanTaskAction;
      note?: string;
      idempotencyKey?: string;
      response?: unknown;
    },
  ): Promise<HumanTaskResolutionResult> {
    return this.post(
      "resolveHumanTask",
      `/api/human-tasks/${encodeURIComponent(taskId)}/resolve`,
      input,
    );
  }

  async claimHumanTask(
    taskId: string,
    expectedRevision: number,
  ): Promise<HumanTask> {
    return this.post(
      "claimHumanTask",
      `/api/human-tasks/${encodeURIComponent(taskId)}/claim`,
      { expectedRevision },
    );
  }

  async assignHumanTask(
    taskId: string,
    input: { expectedRevision: number; assignee?: string },
  ): Promise<HumanTask> {
    return this.post(
      "assignHumanTask",
      `/api/human-tasks/${encodeURIComponent(taskId)}/assign`,
      input,
    );
  }

  async getContributionHosts(
    threadId: string,
  ): Promise<ContributionHostSnapshot> {
    return this.get(
      "getContributionHosts",
      `/api/threads/${encodeURIComponent(threadId)}/contribution-hosts`,
    );
  }

  async selectPreviewHandler(
    threadId: string,
    input: { path?: string; contentType?: string },
  ): Promise<MediaHandlerSelection> {
    return this.get(
      "selectPreviewHandler",
      `/api/threads/${encodeURIComponent(threadId)}/preview-handler${queryString(input)}`,
    );
  }

  async selectContextLoader(
    threadId: string,
    input: { path?: string; contentType?: string },
  ): Promise<MediaHandlerSelection> {
    return this.get(
      "selectContextLoader",
      `/api/threads/${encodeURIComponent(threadId)}/context-loader${queryString(input)}`,
    );
  }

  async invokeMediaHandler(
    threadId: string,
    input: {
      operation: MediaHandlerOperation;
      contributionId?: string;
      path?: string;
      resourceId?: string;
      contentType?: string;
      options?: Record<string, unknown>;
    },
  ): Promise<MediaHandlerInvocationResponse> {
    return this.post(
      "invokeMediaHandler",
      `/api/threads/${encodeURIComponent(threadId)}/media-handlers/invoke`,
      { ...input, options: input.options ?? {} },
    );
  }

  async startPluginAppSession(
    threadId: string,
    contributionId: string,
  ): Promise<AppViewSessionResponse> {
    return this.post(
      "startPluginAppSession",
      `/api/threads/${encodeURIComponent(threadId)}/plugin-app-sessions`,
      { contributionId },
    );
  }

  async getPluginAppContent(
    threadId: string,
    sessionId: string,
  ): Promise<string> {
    const response = await fetch(
      `${this.baseUrl}/api/threads/${encodeURIComponent(threadId)}/plugin-app-sessions/${encodeURIComponent(sessionId)}/content`,
      { headers: this.authHeaders() },
    );
    if (!response.ok) {
      const message = await response.text();
      throw new ApiResponseError(
        response.status,
        message || `${response.status} ${response.statusText}`,
      );
    }
    return response.text();
  }

  async postPluginAppMessage(
    threadId: string,
    sessionId: string,
    channel: string,
    payload: unknown,
  ): Promise<AppViewMessage> {
    return this.post(
      "postPluginAppMessage",
      `/api/threads/${encodeURIComponent(threadId)}/plugin-app-sessions/${encodeURIComponent(sessionId)}/messages`,
      { channel, payload },
    );
  }

  async stopPluginAppSession(
    threadId: string,
    sessionId: string,
  ): Promise<AppViewSession> {
    return this.delete(
      "stopPluginAppSession",
      `/api/threads/${encodeURIComponent(threadId)}/plugin-app-sessions/${encodeURIComponent(sessionId)}`,
    );
  }
}
