import type {
  AgentEvent,
  AgentListItem,
  BrowserOutput,
  BrowserRuntimeRoute,
  BrowserRuntimeStatus,
  CollaborationMode,
  ComputerObservation,
  ComputerWindowTarget,
  ExperienceMode,
  GoalSnapshot,
  GoalStatus,
  InlineImageAttachment,
  InlineMessageContentPart,
  LibraryProviderId,
  LocalGitOperation,
  LocalGitResponse,
  Message,
  Project,
  ProviderHealthCheckResult,
  ProviderModelSyncResult,
  ScmRemoteConnectorResponse,
  Thread,
  ThreadModelSelection,
  ToolResult,
  TurnCancelResult,
  TurnStatus,
} from "../../types";
import {
  conversationSendTrace,
  type ConversationSendTraceContext,
} from "../../conversationSendTrace";
import { recordConversationSendTrace } from "../../platform";
import { ExtensionsApi } from "./extensions";
import { parseResponse, queryString } from "./transport";

export type MessageHistoryCursor = Pick<Message, "createdAt" | "id">;

export type MessageHistoryPage = {
  after?: MessageHistoryCursor;
  before?: MessageHistoryCursor;
  limit: number;
};

export type ConversationEventPage = {
  before?: number;
  limit: number;
};

export class ConversationApi extends ExtensionsApi {
  async executeLocalGit(
    threadId: string,
    repository: string,
    operation: LocalGitOperation,
  ): Promise<LocalGitResponse> {
    return this.post(
      "executeLocalGit",
      `/api/threads/${encodeURIComponent(threadId)}/local-git/v1`,
      { repository, operation },
    );
  }

  async getScmRemoteConnector(
    threadId: string,
    remoteName: string,
  ): Promise<ScmRemoteConnectorResponse> {
    return this.get(
      "getScmRemoteConnector",
      `/api/threads/${encodeURIComponent(threadId)}/scm/remotes/${encodeURIComponent(remoteName)}/connector`,
    );
  }

  async setScmRemoteConnector(
    threadId: string,
    remoteName: string,
    input: {
      connectorPluginId: string | null;
      connectorId: string | null;
      accountBindingId?: string | null;
    },
  ): Promise<ScmRemoteConnectorResponse> {
    return this.put(
      "setScmRemoteConnector",
      `/api/threads/${encodeURIComponent(threadId)}/scm/remotes/${encodeURIComponent(remoteName)}/connector`,
      input,
    );
  }

  async testProviderConnection(
    providerId?: string,
  ): Promise<ProviderHealthCheckResult> {
    return this.post("testProviderConnection", "/api/provider/test", {
      providerId,
    });
  }

  /** Refreshes the cached model list for one connection from its API. */
  async syncProviderModels(
    providerId: string,
  ): Promise<ProviderModelSyncResult> {
    return this.post(
      "syncProviderModels",
      `/api/provider/${encodeURIComponent(providerId)}/models/sync`,
      {},
    );
  }

  /** Pins a model to a thread. Pass `null` to follow the active connection. */
  async setThreadModel(
    threadId: string,
    selection: ThreadModelSelection | null,
  ): Promise<Thread> {
    return this.put("setThreadModel", `/api/threads/${threadId}/model`, {
      selection,
    });
  }

  async listProjects(): Promise<Project[]> {
    return this.get("listProjects", "/api/projects");
  }

  async createProject(input: {
    name: string;
    workspaceRoot?: string | null;
    pinned?: boolean;
    sortOrder?: number;
  }): Promise<Project> {
    return this.post("createProject", "/api/projects", input);
  }

  async updateProject(
    projectId: string,
    input: {
      name?: string;
      workspaceRoot?: string | null;
      pinned?: boolean;
      sortOrder?: number;
    },
  ): Promise<Project> {
    return this.patch("updateProject", `/api/projects/${projectId}`, input);
  }

  async deleteProject(projectId: string): Promise<void> {
    return this.delete("deleteProject", `/api/projects/${projectId}`);
  }

  async listThreads(
    includeArchived = false,
    experienceMode?: ExperienceMode,
  ): Promise<Thread[]> {
    return this.get(
      "listThreads",
      `/api/threads${queryString({
        includeArchived: includeArchived ? "true" : undefined,
        experienceMode,
      })}`,
    );
  }

  async createThread(input: {
    title?: string;
    workspaceRoot?: string;
    projectId?: string;
    experienceMode?: ExperienceMode;
  }): Promise<Thread> {
    return this.post("createThread", "/api/threads", input);
  }

  async generateThreadTitle(
    threadId: string,
    prompt: string,
    expectedTitle: string,
  ): Promise<{ thread: Thread; updated: boolean }> {
    return this.post("generateThreadTitle", `/api/threads/${threadId}/title`, {
      prompt,
      expectedTitle,
    });
  }

  async updateThread(
    threadId: string,
    input: {
      title?: string;
      projectId?: string | null;
      archivedAt?: string | null;
    },
  ): Promise<Thread> {
    return this.patch("updateThread", `/api/threads/${threadId}`, input);
  }

  async deleteThread(threadId: string): Promise<void> {
    return this.delete("deleteThread", `/api/threads/${threadId}`);
  }

  async listMessages(
    threadId: string,
    signal?: AbortSignal,
    page?: MessageHistoryPage,
  ): Promise<Message[]> {
    return this.get(
      "listMessages",
      `/api/threads/${threadId}/messages${queryString({
        afterCreatedAt: page?.after?.createdAt,
        afterId: page?.after?.id,
        beforeCreatedAt: page?.before?.createdAt,
        beforeId: page?.before?.id,
        limit: page?.limit,
      })}`,
      signal,
    );
  }

  async sendMessage(
    threadId: string,
    content: string,
    sourcePaths: string[] = [],
    skillIds: string[] = [],
    collaborationMode: CollaborationMode = "default",
    goalId?: string,
    imageAttachments: InlineImageAttachment[] = [],
    contentParts: InlineMessageContentPart[] = [],
    libraryProvider?: LibraryProviderId,
    trace?: ConversationSendTraceContext,
  ): Promise<{
    message: Message;
    turnId: string | null;
    queued: boolean;
  }> {
    if (trace) {
      recordConversationSendTrace(
        conversationSendTrace(trace, "fetch_started"),
      );
    }
    const response = await fetch(
      `${this.baseUrl}/api/threads/${threadId}/messages`,
      {
        method: "POST",
        headers: {
          ...this.authHeaders(true),
          ...(trace
            ? {
                "x-opentopia-request-id": trace.requestId,
                "x-opentopia-client-started-at-ms": String(
                  trace.clientStartedAtMs,
                ),
              }
            : {}),
        },
        body: JSON.stringify({
          content,
          sourcePaths,
          skillIds,
          collaborationMode,
          goalId,
          imageAttachments,
          contentParts,
          libraryProvider,
        }),
      },
    );
    if (trace) {
      recordConversationSendTrace(
        conversationSendTrace(trace, "response_headers", {
          httpStatus: response.status,
          serverDurationMs: numericHeader(
            response,
            "x-opentopia-server-duration-ms",
          ),
          clientToServerMs: numericHeader(
            response,
            "x-opentopia-client-to-server-ms",
          ),
        }),
      );
    }
    const turnId = response.headers.get("x-opentopia-turn-id");
    const queued = response.headers.get("x-opentopia-queued") === "true";
    const message = await parseResponse<Message>(response, "sendMessage");
    if (trace) {
      recordConversationSendTrace(
        conversationSendTrace(trace, "response_parsed", {
          turnId,
          messageId: message.id,
          queued,
          httpStatus: response.status,
        }),
      );
    }
    return {
      message,
      turnId,
      queued,
    };
  }

  async getGoal(
    threadId: string,
    signal?: AbortSignal,
  ): Promise<GoalSnapshot | null> {
    return this.get("getGoal", `/api/threads/${threadId}/goal`, signal);
  }

  async updateGoalStatus(
    threadId: string,
    goalId: string,
    status: GoalStatus,
  ): Promise<GoalSnapshot> {
    return this.updateGoal(threadId, goalId, { status });
  }

  async updateGoal(
    threadId: string,
    goalId: string,
    update: {
      status?: GoalStatus;
      objective?: string;
      constraints?: string[];
      acceptance?: string[];
    },
  ): Promise<GoalSnapshot> {
    return this.patch(
      "updateGoal",
      `/api/threads/${threadId}/goal/${goalId}`,
      update,
    );
  }

  async resumeExternalAction(
    threadId: string,
    turnId: string,
    observation?: string,
  ): Promise<{
    accepted: boolean;
    resumed: boolean;
    turnId: string;
    invocationId: number;
  }> {
    return this.post(
      "resumeExternalAction",
      `/api/threads/${threadId}/turns/${turnId}/external-action/resume`,
      { observation },
    );
  }

  async runBrowserCommand(
    threadId: string,
    input: {
      action:
        | "navigate"
        | "observe"
        | "screenshot"
        | "click"
        | "type"
        | "select"
        | "hover"
        | "scroll"
        | "switch_target"
        | "wait"
        | "download"
        | "close";
      url?: string;
      selector?: string;
      observationId?: string;
      nodeRef?: string;
      text?: string;
      value?: string;
      clearFirst?: boolean;
      deltaX?: number;
      deltaY?: number;
      targetRef?: string;
      includeScreenshot?: boolean;
      condition?: "document_complete" | "selector" | "text";
      timeoutMs?: number;
      expectedFilename?: string;
    },
  ): Promise<BrowserOutput> {
    return this.post(
      "runBrowserCommand",
      `/api/threads/${threadId}/browser`,
      input,
    );
  }

  async getBrowserRuntime(threadId: string): Promise<BrowserRuntimeStatus> {
    return this.get(
      "getBrowserRuntime",
      `/api/threads/${threadId}/browser/runtime`,
    );
  }

  async bindBrowserRuntime(
    threadId: string,
    route: BrowserRuntimeRoute,
  ): Promise<BrowserRuntimeStatus> {
    return this.post(
      "bindBrowserRuntime",
      `/api/threads/${threadId}/browser/runtime`,
      { route },
    );
  }

  async listComputerWindows(threadId: string): Promise<ComputerWindowTarget[]> {
    return this.get(
      "listComputerWindows",
      `/api/threads/${threadId}/computer/windows`,
    );
  }

  async observeComputerWindow(
    threadId: string,
    windowId: string,
  ): Promise<ComputerObservation> {
    return this.post(
      "observeComputerWindow",
      `/api/threads/${threadId}/computer/observe`,
      { windowId },
    );
  }

  async closeComputerSession(threadId: string): Promise<void> {
    await this.post(
      "closeComputerSession",
      `/api/threads/${threadId}/computer/session`,
      {},
    );
  }

  async getTurnStatus(
    threadId: string,
    signal?: AbortSignal,
  ): Promise<TurnStatus | null> {
    return this.get("getTurnStatus", `/api/threads/${threadId}/turn`, signal);
  }

  async listActivityStatuses(signal?: AbortSignal): Promise<TurnStatus[]> {
    return this.get("listActivityStatuses", "/api/activity/statuses", signal);
  }

  async listAgents(
    threadId: string,
    signal?: AbortSignal,
  ): Promise<AgentListItem[]> {
    return this.get("listAgents", `/api/threads/${threadId}/agents`, signal);
  }

  async interruptAgent(threadId: string, agentThreadId: string): Promise<void> {
    return this.post(
      "interruptAgent",
      `/api/threads/${threadId}/agents/${agentThreadId}/interrupt`,
      {},
    );
  }

  async cancelTurn(
    threadId: string,
    turnId?: string,
  ): Promise<TurnCancelResult> {
    return this.post("cancelTurn", `/api/threads/${threadId}/turn/cancel`, {
      turnId,
    });
  }

  async listEvents(
    threadId: string,
    since?: number,
    signal?: AbortSignal,
  ): Promise<AgentEvent[]> {
    return this.get(
      "listEvents",
      `/api/threads/${threadId}/events${queryString({ since })}`,
      signal,
    );
  }

  async listConversationEvents(
    threadId: string,
    since?: number,
    signal?: AbortSignal,
    page?: ConversationEventPage,
  ): Promise<AgentEvent[]> {
    return this.get(
      "listConversationEvents",
      `/api/threads/${threadId}/events${queryString({
        since,
        before: page?.before,
        limit: page?.limit,
        view: "conversation",
      })}`,
      signal,
    );
  }

  async getToolResultDetail(
    threadId: string,
    eventId: string,
    signal?: AbortSignal,
  ): Promise<ToolResult> {
    return this.get(
      "getToolResultDetail",
      `/api/threads/${encodeURIComponent(threadId)}/events/${encodeURIComponent(eventId)}/tool-result`,
      signal,
    );
  }
}

function numericHeader(response: Response, name: string): number | undefined {
  const value = response.headers.get(name);
  if (value === null) return undefined;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : undefined;
}
