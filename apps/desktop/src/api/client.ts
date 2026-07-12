import type {
  AgentEvent,
  AppSettings,
  ArtifactContent,
  ArtifactDescriptor,
  ContextStatus,
  ContextSummary,
  DiffFileActionResult,
  McpCallResult,
  McpServerStatus,
  McpServerView,
  Message,
  PermissionMode,
  ProviderHealth,
  ProviderHealthCheckResult,
  ProviderKind,
  SandboxDescriptor,
  TerminalCancelResponse,
  TerminalEvent,
  TerminalStartResponse,
  TerminalSession,
  Thread,
  TurnCancelResult,
  TurnStatus,
  ThreadMcpServer,
  ThreadMcpServerView,
  WorkspaceDiff,
  WorkspaceDiffHunk,
  WorkspaceDiffHunkAction,
  WorkspaceFilePreview,
  WorkspaceTree,
} from "../types";
import { getLoadedApiToken } from "../platform";

export type StreamHandle = { close(): void };

export class ApiClient {
  private readonly apiToken: string;

  constructor(
    private readonly baseUrl: string,
    apiToken?: string,
  ) {
    this.apiToken = apiToken || getLoadedApiToken();
  }

  async health(): Promise<{
    ok: boolean;
    service: string;
    apiVersion: number;
  }> {
    return this.get("/health");
  }

  async getSettings(): Promise<AppSettings> {
    return this.get("/api/settings");
  }

  async updateSettings(input: {
    providers?: {
      id: string;
      kind: ProviderKind;
      baseUrl: string;
      model: string;
      apiKeySource: string;
      apiKeyConfigured: boolean;
      healthStatus?: string | null;
    }[];
    activeProviderId?: string;
    providerKind?: ProviderKind;
    baseUrl?: string;
    model?: string;
    apiKeySource?: string;
    permissionMode?: PermissionMode;
    defaultWorkspaceRoot?: string;
    clearDefaultWorkspaceRoot?: boolean;
  }): Promise<AppSettings> {
    return this.patch("/api/settings", input);
  }

  async getProviderHealth(): Promise<ProviderHealth[]> {
    return this.get("/api/provider/health");
  }

  async testProviderConnection(
    providerId?: string,
  ): Promise<ProviderHealthCheckResult> {
    return this.post("/api/provider/test", { providerId });
  }

  async listThreads(): Promise<Thread[]> {
    return this.get("/api/threads");
  }

  async createThread(input: {
    title?: string;
    workspaceRoot?: string;
  }): Promise<Thread> {
    return this.post("/api/threads", input);
  }

  async listMessages(threadId: string): Promise<Message[]> {
    return this.get(`/api/threads/${threadId}/messages`);
  }

  async sendMessage(threadId: string, content: string): Promise<Message> {
    return this.post(`/api/threads/${threadId}/messages`, { content });
  }

  async getTurnStatus(threadId: string): Promise<TurnStatus | null> {
    return this.get(`/api/threads/${threadId}/turn`);
  }

  async cancelTurn(
    threadId: string,
    turnId?: string,
  ): Promise<TurnCancelResult> {
    return this.post(`/api/threads/${threadId}/turn/cancel`, { turnId });
  }

  async listEvents(threadId: string, since?: number): Promise<AgentEvent[]> {
    const query = since ? `?since=${since}` : "";
    return this.get(`/api/threads/${threadId}/events${query}`);
  }

  async startTerminalCommand(
    threadId: string,
    command: string,
    options?: { cwd?: string; timeoutMs?: number },
  ): Promise<TerminalStartResponse> {
    return this.post(`/api/threads/${threadId}/terminal/commands`, {
      command,
      cwd: options?.cwd,
      timeoutMs: options?.timeoutMs,
    });
  }

  async cancelTerminalCommand(
    threadId: string,
    commandId?: string,
  ): Promise<TerminalCancelResponse> {
    return this.post(`/api/threads/${threadId}/terminal/cancel`, {
      commandId,
    });
  }

  async listTerminalHistory(
    threadId: string,
    since?: number,
  ): Promise<TerminalEvent[]> {
    return this.get(
      `/api/threads/${threadId}/terminal/history${queryString({ since })}`,
    );
  }

  async getTerminalSession(threadId: string): Promise<TerminalSession | null> {
    return this.get(`/api/threads/${threadId}/terminal/session`);
  }

  async ensureTerminalSession(
    threadId: string,
    options?: { cwd?: string; cols?: number; rows?: number },
  ): Promise<TerminalSession> {
    return this.post(
      `/api/threads/${threadId}/terminal/session`,
      options ?? {},
    );
  }

  async writeTerminalSession(
    threadId: string,
    sessionId: string,
    data: string,
  ): Promise<TerminalSession> {
    return this.post(`/api/threads/${threadId}/terminal/session/input`, {
      sessionId,
      data,
    });
  }

  async resizeTerminalSession(
    threadId: string,
    sessionId: string,
    cols: number,
    rows: number,
  ): Promise<TerminalSession> {
    return this.post(`/api/threads/${threadId}/terminal/session/resize`, {
      sessionId,
      cols,
      rows,
    });
  }

  async closeTerminalSession(
    threadId: string,
    sessionId: string,
  ): Promise<TerminalSession> {
    return this.post(`/api/threads/${threadId}/terminal/session/close`, {
      sessionId,
    });
  }

  async decideApproval(
    threadId: string,
    approvalId: string,
    approved: boolean,
  ): Promise<{ accepted: boolean; executed: boolean }> {
    return this.post(
      `/api/threads/${threadId}/approvals/${approvalId}/decision`,
      { approved },
    );
  }

  async listPendingApprovals(
    threadId: string,
  ): Promise<Array<{ approvalId: string }>> {
    return this.get(`/api/threads/${threadId}/approvals?status=pending`);
  }

  async listWorkspaceTree(
    threadId: string,
    path?: string,
  ): Promise<WorkspaceTree> {
    return this.get(
      `/api/threads/${threadId}/workspace/tree${queryString({ path })}`,
    );
  }

  async readWorkspaceFile(
    threadId: string,
    path: string,
  ): Promise<WorkspaceFilePreview> {
    return this.get(
      `/api/threads/${threadId}/workspace/file${queryString({ path })}`,
    );
  }

  async getWorkspaceDiff(threadId: string): Promise<WorkspaceDiff> {
    return this.get(`/api/threads/${threadId}/workspace/diff`);
  }

  async revertWorkspaceFile(
    threadId: string,
    path: string,
    confirm: boolean,
  ): Promise<DiffFileActionResult> {
    return this.post(`/api/threads/${threadId}/workspace/diff/revert`, {
      path,
      confirm,
    });
  }

  async applyWorkspaceDiffHunk(
    threadId: string,
    hunk: WorkspaceDiffHunk,
    action: WorkspaceDiffHunkAction,
    confirm: boolean,
  ): Promise<DiffFileActionResult> {
    return this.post(`/api/threads/${threadId}/workspace/diff/hunk`, {
      path: hunk.path,
      scope: hunk.scope,
      patch: hunk.patch ?? hunk.raw,
      action,
      confirm,
    });
  }

  async getSandbox(threadId: string): Promise<SandboxDescriptor> {
    return this.get(`/api/threads/${threadId}/sandbox`);
  }

  async getContextStatus(threadId: string): Promise<ContextStatus> {
    return this.get(`/api/threads/${threadId}/context`);
  }

  async compactContext(
    threadId: string,
    summary?: string,
  ): Promise<ContextSummary> {
    return this.post(`/api/threads/${threadId}/context/compact`, { summary });
  }

  async listArtifacts(threadId: string): Promise<ArtifactDescriptor[]> {
    return this.get(`/api/threads/${threadId}/artifacts`);
  }

  async getArtifact(
    threadId: string,
    artifactId: string,
  ): Promise<ArtifactContent> {
    const artifact = await this.get<{
      id: string;
      storage:
        { type: "inline"; content: string } | { type: "path"; path: string };
    }>(`/api/threads/${threadId}/artifacts/${artifactId}`);
    if (artifact.storage.type === "inline") {
      return {
        id: artifact.id,
        content: artifact.storage.content,
        metadata: (artifact as { metadata?: unknown }).metadata,
      };
    }
    return {
      id: artifact.id,
      content: `Artifact is stored on disk:\n${artifact.storage.path}`,
      filePath: artifact.storage.path,
      metadata: (artifact as { metadata?: unknown }).metadata,
    };
  }

  async listMcpServers(): Promise<McpServerView[]> {
    return this.get("/api/mcp/servers");
  }

  async createMcpServer(input: {
    name: string;
    command: string;
    args?: string[];
    cwd?: string;
    envKeys?: string[];
    timeoutMs?: number;
    enabled?: boolean;
  }): Promise<McpServerView> {
    return this.post("/api/mcp/servers", input);
  }

  async listThreadMcpServers(threadId: string): Promise<ThreadMcpServerView[]> {
    return this.get(`/api/threads/${threadId}/mcp`);
  }

  async setThreadMcpServer(
    threadId: string,
    serverId: string,
    enabled: boolean,
  ): Promise<ThreadMcpServer> {
    return this.put(`/api/threads/${threadId}/mcp/${serverId}`, { enabled });
  }

  async callMcpTool(
    serverId: string,
    toolName: string,
    args: unknown,
    threadId?: string,
  ): Promise<McpCallResult> {
    return this.post(`/api/mcp/servers/${serverId}/call-tool`, {
      toolName,
      arguments: args,
      threadId,
    });
  }

  async restartMcpServer(serverId: string): Promise<McpServerStatus> {
    return this.post(`/api/mcp/servers/${serverId}/restart`, {});
  }

  openEventStream(
    threadId: string,
    since: number | undefined,
    onEvent: (event: AgentEvent) => void,
  ): StreamHandle {
    const query = since ? `?since=${since}` : "";
    return this.openAuthenticatedSse(
      `/api/threads/${threadId}/events/stream${query}`,
      (data) => onEvent(JSON.parse(data) as AgentEvent),
    );
  }

  openTerminalStream(
    threadId: string,
    since: number | undefined,
    onEvent: (event: TerminalEvent) => void,
  ): StreamHandle {
    const query = since ? `?since=${since}` : "";
    return this.openAuthenticatedSse(
      `/api/threads/${threadId}/terminal/stream${query}`,
      (data) => onEvent(JSON.parse(data) as TerminalEvent),
    );
  }

  private async get<T>(path: string): Promise<T> {
    const response = await fetch(`${this.baseUrl}${path}`, {
      headers: this.authHeaders(),
    });
    return parseResponse<T>(response);
  }

  private async post<T>(path: string, body: unknown): Promise<T> {
    const response = await fetch(`${this.baseUrl}${path}`, {
      method: "POST",
      headers: this.authHeaders(true),
      body: JSON.stringify(body),
    });
    return parseResponse<T>(response);
  }

  private async patch<T>(path: string, body: unknown): Promise<T> {
    const response = await fetch(`${this.baseUrl}${path}`, {
      method: "PATCH",
      headers: this.authHeaders(true),
      body: JSON.stringify(body),
    });
    return parseResponse<T>(response);
  }

  private async put<T>(path: string, body: unknown): Promise<T> {
    const response = await fetch(`${this.baseUrl}${path}`, {
      method: "PUT",
      headers: this.authHeaders(true),
      body: JSON.stringify(body),
    });
    return parseResponse<T>(response);
  }

  private authHeaders(json = false): HeadersInit {
    return {
      authorization: `Bearer ${this.apiToken}`,
      ...(json ? { "content-type": "application/json" } : {}),
    };
  }

  private openAuthenticatedSse(
    path: string,
    onData: (data: string) => void,
  ): StreamHandle {
    const controller = new AbortController();
    let lastSequence = readSince(path);

    const run = async () => {
      while (!controller.signal.aborted) {
        try {
          const response = await fetch(
            withSince(`${this.baseUrl}${path}`, lastSequence),
            {
              headers: {
                ...this.authHeaders(),
                accept: "text/event-stream",
              },
              cache: "no-store",
              signal: controller.signal,
            },
          );
          if (!response.ok) {
            throw new Error(
              `Event stream failed: ${response.status} ${response.statusText}`,
            );
          }
          if (!response.body)
            throw new Error("Event stream response has no body");

          await consumeSse(
            response.body,
            (data) => {
              try {
                const sequence = JSON.parse(data)?.seq;
                if (typeof sequence === "number") lastSequence = sequence;
              } catch {
                // Event payload validation remains the caller's responsibility.
              }
              onData(data);
            },
            controller.signal,
          );
        } catch (error) {
          if (controller.signal.aborted) break;
          console.error("OpenTopia event stream disconnected", error);
        }
        if (!controller.signal.aborted)
          await abortableDelay(1_000, controller.signal);
      }
    };

    void run();
    return { close: () => controller.abort() };
  }
}

async function consumeSse(
  body: ReadableStream<Uint8Array>,
  onData: (data: string) => void,
  signal: AbortSignal,
): Promise<void> {
  const reader = body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  try {
    while (!signal.aborted) {
      const { done, value } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      buffer = buffer.replace(/\r\n/g, "\n");
      let boundary = buffer.indexOf("\n\n");
      while (boundary >= 0) {
        const frame = buffer.slice(0, boundary);
        buffer = buffer.slice(boundary + 2);
        const data = frame
          .split("\n")
          .filter((line) => line.startsWith("data:"))
          .map((line) => line.slice(5).trimStart())
          .join("\n");
        if (data) onData(data);
        boundary = buffer.indexOf("\n\n");
      }
    }
  } finally {
    reader.releaseLock();
  }
}

function readSince(path: string): number | undefined {
  const query = path.split("?", 2)[1];
  const value = query ? new URLSearchParams(query).get("since") : null;
  const parsed = value ? Number(value) : Number.NaN;
  return Number.isFinite(parsed) ? parsed : undefined;
}

function withSince(url: string, since: number | undefined): string {
  if (since === undefined) return url;
  const parsed = new URL(url);
  parsed.searchParams.set("since", String(since));
  return parsed.toString();
}

function abortableDelay(
  milliseconds: number,
  signal: AbortSignal,
): Promise<void> {
  return new Promise((resolve) => {
    const timeout = window.setTimeout(resolve, milliseconds);
    signal.addEventListener(
      "abort",
      () => {
        window.clearTimeout(timeout);
        resolve();
      },
      { once: true },
    );
  });
}

async function parseResponse<T>(response: Response): Promise<T> {
  if (!response.ok) {
    const text = await response.text();
    throw new Error(text || `${response.status} ${response.statusText}`);
  }
  return response.json() as Promise<T>;
}

function queryString(
  values: Record<string, string | number | undefined>,
): string {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(values)) {
    if (value !== undefined && value !== "") params.set(key, String(value));
  }
  const query = params.toString();
  return query ? `?${query}` : "";
}
