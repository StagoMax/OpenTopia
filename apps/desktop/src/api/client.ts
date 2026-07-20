import type {
  AgentEvent,
  AppSettings,
  ArtifactContent,
  ArtifactDescriptor,
  BrowserOutput,
  ContextStatus,
  ContextSummary,
  DiffFileActionResult,
  ExperienceMode,
  GitBranchInfo,
  GitStatusSummary,
  GitWorkflowAction,
  GitWorkflowResponse,
  McpCallResult,
  McpServerInput,
  McpServerStatus,
  McpServerView,
  Message,
  PermissionMode,
  PreviewDescriptor,
  PreviewTarget,
  Project,
  ProviderHealth,
  ProviderHealthCheckResult,
  ProviderKind,
  ProviderSettings,
  SandboxDescriptor,
  SkillDescriptor,
  SpreadsheetPreview,
  SpreadsheetPreviewRange,
  SubagentRun,
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

export class ApiResponseError extends Error {
  constructor(
    readonly status: number,
    message: string,
  ) {
    super(message);
    this.name = "ApiResponseError";
  }
}

type PreviewDescriptorResponse = {
  id: string;
  source: "workspace" | "artifact";
  path?: string | null;
  name: string;
  kind: "text" | "image" | "pdf" | "spreadsheet" | "unsupported";
  contentType: string;
  bytes: number;
  readonly: boolean;
  revision: string;
};

type SpreadsheetWorkbookResponse = {
  previewId: string;
  sheets: Array<{
    name: string;
    kind: string;
    visibility: "visible" | "hidden" | "very_hidden";
    rowCount: number;
    columnCount: number;
  }>;
};

type SpreadsheetRangeResponse = {
  previewId: string;
  sheet: string;
  range: {
    start: { row: number; column: number };
    end: { row: number; column: number };
  };
  rows: Array<
    Array<{
      value: { type: string; value?: unknown };
      formula?: string | null;
    }>
  >;
};

export class ApiClient {
  private readonly apiToken: string;
  private readonly legacyPreviewContent = new Map<string, Blob>();

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
    providers?: ProviderSettings[];
    activeProviderId?: string;
    providerKind?: ProviderKind;
    baseUrl?: string;
    model?: string;
    apiKeySource?: string;
    permissionMode?: PermissionMode;
    defaultWorkspaceRoot?: string;
    clearDefaultWorkspaceRoot?: boolean;
    sandbox?: AppSettings["sandbox"];
  }): Promise<AppSettings> {
    return this.patch("/api/settings", input);
  }

  async getProviderHealth(): Promise<ProviderHealth[]> {
    return this.get("/api/provider/health");
  }

  async listSkills(workspaceRoot?: string | null): Promise<SkillDescriptor[]> {
    return this.get(
      `/api/skills${queryString({ workspaceRoot: workspaceRoot ?? undefined })}`,
    );
  }

  async testProviderConnection(
    providerId?: string,
  ): Promise<ProviderHealthCheckResult> {
    return this.post("/api/provider/test", { providerId });
  }

  async listProjects(): Promise<Project[]> {
    return this.get("/api/projects");
  }

  async createProject(input: {
    name: string;
    workspaceRoot?: string | null;
    pinned?: boolean;
    sortOrder?: number;
  }): Promise<Project> {
    return this.post("/api/projects", input);
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
    return this.patch(`/api/projects/${projectId}`, input);
  }

  async deleteProject(projectId: string): Promise<void> {
    return this.delete(`/api/projects/${projectId}`);
  }

  async listThreads(includeArchived = false): Promise<Thread[]> {
    return this.get(
      `/api/threads${queryString({
        includeArchived: includeArchived ? "true" : undefined,
      })}`,
    );
  }

  async createThread(input: {
    title?: string;
    workspaceRoot?: string;
    projectId?: string;
    experienceMode?: ExperienceMode;
  }): Promise<Thread> {
    return this.post("/api/threads", input);
  }

  async updateThread(
    threadId: string,
    input: {
      title?: string;
      projectId?: string | null;
      archivedAt?: string | null;
    },
  ): Promise<Thread> {
    return this.patch(`/api/threads/${threadId}`, input);
  }

  async deleteThread(threadId: string): Promise<void> {
    return this.delete(`/api/threads/${threadId}`);
  }

  async listMessages(threadId: string): Promise<Message[]> {
    return this.get(`/api/threads/${threadId}/messages`);
  }

  async sendMessage(
    threadId: string,
    content: string,
    sourcePaths: string[] = [],
    skillIds: string[] = [],
  ): Promise<{ message: Message; turnId: string | null }> {
    const response = await fetch(
      `${this.baseUrl}/api/threads/${threadId}/messages`,
      {
        method: "POST",
        headers: this.authHeaders(true),
        body: JSON.stringify({ content, sourcePaths, skillIds }),
      },
    );
    const turnId = response.headers.get("x-opentopia-turn-id");
    return {
      message: await parseResponse<Message>(response),
      turnId,
    };
  }

  async runBrowserCommand(
    threadId: string,
    input: {
      action:
        | "navigate"
        | "snapshot"
        | "screenshot"
        | "click"
        | "type"
        | "wait"
        | "download"
        | "close";
      url?: string;
      selector?: string;
      text?: string;
      clearFirst?: boolean;
      condition?: "document_complete" | "selector" | "text";
      timeoutMs?: number;
      expectedFilename?: string;
    },
  ): Promise<BrowserOutput> {
    return this.post(`/api/threads/${threadId}/browser`, input);
  }

  async getTurnStatus(threadId: string): Promise<TurnStatus | null> {
    return this.get(`/api/threads/${threadId}/turn`);
  }

  async listSubagents(threadId: string): Promise<SubagentRun[]> {
    return this.get(`/api/threads/${threadId}/subagents`);
  }

  async spawnSubagent(
    threadId: string,
    input: {
      name: string;
      input: string;
      agentType?: string;
      forkTurns?: string;
      parentTurnId?: string;
      depth?: number;
    },
  ): Promise<SubagentRun> {
    return this.post(`/api/threads/${threadId}/subagents`, input);
  }

  async sendSubagentInput(
    threadId: string,
    runId: string,
    input: string,
  ): Promise<void> {
    return this.post(`/api/threads/${threadId}/subagents/${runId}/input`, {
      input,
    });
  }

  async cancelSubagent(threadId: string, runId: string): Promise<void> {
    return this.post(`/api/threads/${threadId}/subagents/${runId}/cancel`, {});
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

  async runGitWorkflow(
    threadId: string,
    action: GitWorkflowAction,
  ): Promise<GitWorkflowResponse> {
    const result = await this.post<GitWorkflowResponse>(
      `/api/threads/${threadId}/git`,
      action,
    );
    if (!result.success) throw new Error(gitFailureMessage(result));
    return result;
  }

  async getGitStatus(threadId: string): Promise<GitStatusSummary> {
    const result = await this.runGitWorkflow(threadId, {
      type: "status",
      request: { includeUntracked: true },
    });
    return parseGitStatus(result.stdout);
  }

  async listGitBranches(threadId: string): Promise<GitBranchInfo[]> {
    const result = await this.runGitWorkflow(threadId, {
      type: "list_branches",
      request: { includeRemote: true },
    });
    return parseGitBranches(result.stdout);
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

  async resolvePreview(
    threadId: string,
    target: PreviewTarget,
  ): Promise<PreviewDescriptor> {
    if (target.type === "url") {
      return {
        id: `web:${threadId}`,
        threadId,
        target,
        renderer: "web",
        title: target.url || "Browser",
        contentType: "text/html",
        revision: target.url,
        readonly: true,
      };
    }

    try {
      const response = await this.post<PreviewDescriptorResponse>(
        `/api/threads/${threadId}/previews/resolve`,
        target.type === "workspace"
          ? { source: "workspace", path: target.path }
          : { source: "artifact", artifactId: target.artifactId },
      );
      return {
        id: response.id,
        threadId,
        target,
        renderer:
          response.kind === "text"
            ? previewRenderer(response.name, response.contentType)
            : response.kind,
        title: response.name,
        contentType: response.contentType,
        bytes: response.bytes,
        revision: response.revision,
        readonly: response.readonly,
        externalPath:
          response.source === "artifact" ? response.path : undefined,
      };
    } catch (cause) {
      if (cause instanceof ApiResponseError && cause.status === 404) {
        return this.resolveLegacyPreview(threadId, target);
      }
      throw cause;
    }
  }

  async getPreviewContent(threadId: string, previewId: string): Promise<Blob> {
    const legacy = this.legacyPreviewContent.get(previewId);
    if (legacy) return legacy;

    const response = await fetch(
      `${this.baseUrl}/api/threads/${threadId}/previews/${encodeURIComponent(previewId)}/content`,
      { headers: this.authHeaders() },
    );
    if (!response.ok) {
      const message = await response.text();
      throw new Error(
        message ||
          `Preview content failed: ${response.status} ${response.statusText}`,
      );
    }
    return response.blob();
  }

  async getSpreadsheetPreview(
    threadId: string,
    previewId: string,
  ): Promise<SpreadsheetPreview> {
    const workbook = await this.get<SpreadsheetWorkbookResponse>(
      `/api/threads/${threadId}/previews/${encodeURIComponent(previewId)}/workbook`,
    );
    return {
      previewId: workbook.previewId,
      sheets: workbook.sheets.map((sheet) => ({
        id: sheet.name,
        name: sheet.name,
        rowCount: Math.max(1, sheet.rowCount),
        columnCount: Math.max(1, sheet.columnCount),
        hidden: sheet.visibility !== "visible",
      })),
    };
  }

  async getSpreadsheetPreviewRange(
    threadId: string,
    previewId: string,
    sheetId: string,
    input: {
      rowStart: number;
      rowCount: number;
      columnStart: number;
      columnCount: number;
    },
  ): Promise<SpreadsheetPreviewRange> {
    const response = await this.get<SpreadsheetRangeResponse>(
      `/api/threads/${threadId}/previews/${encodeURIComponent(previewId)}/range${queryString(
        {
          sheet: sheetId,
          startRow: input.rowStart,
          rowCount: input.rowCount,
          startColumn: input.columnStart,
          columnCount: input.columnCount,
        },
      )}`,
    );
    const cells = response.rows.flatMap((row, rowOffset) =>
      row.map((cell, columnOffset) => ({
        row: response.range.start.row + rowOffset,
        column: response.range.start.column + columnOffset,
        value: spreadsheetCellValue(cell.value),
        formula: cell.formula,
      })),
    );
    return {
      previewId: response.previewId,
      sheetId: response.sheet,
      rowStart: response.range.start.row,
      columnStart: response.range.start.column,
      rowCount: response.rows.length,
      columnCount: response.rows[0]?.length ?? 0,
      cells,
    };
  }

  async closePreview(previewId: string): Promise<void> {
    this.legacyPreviewContent.delete(previewId);
  }

  async listMcpServers(): Promise<McpServerView[]> {
    return this.get("/api/mcp/servers");
  }

  async createMcpServer(input: McpServerInput): Promise<McpServerView> {
    return this.post("/api/mcp/servers", input);
  }

  async updateMcpServer(
    serverId: string,
    input: McpServerInput,
  ): Promise<McpServerView> {
    return this.patch(`/api/mcp/servers/${serverId}`, {
      ...input,
      clearCwd: !input.cwd,
    });
  }

  async deleteMcpServer(serverId: string): Promise<void> {
    await this.delete(`/api/mcp/servers/${serverId}`);
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
    threadId: string,
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

  private async resolveLegacyPreview(
    threadId: string,
    target: Exclude<PreviewTarget, { type: "url" }>,
  ): Promise<PreviewDescriptor> {
    if (target.type === "workspace") {
      const file = await this.readWorkspaceFile(threadId, target.path);
      const id = `legacy-workspace:${threadId}:${target.path}`;
      const contentType = previewContentType(target.path);
      this.legacyPreviewContent.set(
        id,
        new Blob([file.content], { type: contentType }),
      );
      return {
        id,
        threadId,
        target,
        renderer: previewRenderer(target.path, contentType),
        title: previewTitle(target.path),
        contentType,
        bytes: file.bytes,
        revision: `${file.bytes}:${file.truncated ? "truncated" : "complete"}`,
        readonly: file.readonly,
        truncated: file.truncated,
      };
    }

    const artifact = await this.getArtifact(threadId, target.artifactId);
    const displayPath = artifact.filePath || target.artifactId;
    const id = `legacy-artifact:${threadId}:${target.artifactId}`;
    const contentType = previewContentType(displayPath);
    const hasReadableContent = !artifact.filePath;
    if (hasReadableContent) {
      this.legacyPreviewContent.set(
        id,
        new Blob([artifact.content], { type: contentType }),
      );
    }
    return {
      id,
      threadId,
      target,
      renderer: hasReadableContent
        ? previewRenderer(displayPath, contentType)
        : "unsupported",
      title: previewTitle(displayPath),
      contentType,
      revision: id,
      readonly: true,
      externalPath: artifact.filePath,
    };
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

  private async delete<T>(path: string): Promise<T> {
    const response = await fetch(`${this.baseUrl}${path}`, {
      method: "DELETE",
      headers: this.authHeaders(),
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
    throw new ApiResponseError(
      response.status,
      text || `${response.status} ${response.statusText}`,
    );
  }
  if (response.status === 204) return undefined as T;
  const text = await response.text();
  return (text ? JSON.parse(text) : undefined) as T;
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

function previewTitle(path: string): string {
  const normalized = path.replace(/[\\/]+$/, "");
  return normalized.split(/[\\/]/).at(-1) || path;
}

function previewRenderer(
  path: string,
  contentType: string,
): PreviewDescriptor["renderer"] {
  const extension = path.split(".").at(-1)?.toLocaleLowerCase() ?? "";
  if (contentType.startsWith("image/")) return "image";
  if (contentType === "application/pdf" || extension === "pdf") return "pdf";
  if (["xlsx", "xlsm", "xltx"].includes(extension)) return "spreadsheet";
  if (
    [
      "c",
      "cc",
      "cpp",
      "css",
      "go",
      "h",
      "html",
      "java",
      "js",
      "jsx",
      "json",
      "md",
      "py",
      "rs",
      "sh",
      "toml",
      "ts",
      "tsx",
      "xml",
      "yaml",
      "yml",
    ].includes(extension)
  ) {
    return "code";
  }
  if (contentType.startsWith("text/")) return "text";
  return "unsupported";
}

function previewContentType(path: string): string {
  const extension = path.split(".").at(-1)?.toLocaleLowerCase() ?? "";
  const known: Record<string, string> = {
    bmp: "image/bmp",
    css: "text/css",
    csv: "text/csv",
    gif: "image/gif",
    html: "text/html",
    jpeg: "image/jpeg",
    jpg: "image/jpeg",
    js: "text/javascript",
    json: "application/json",
    md: "text/markdown",
    pdf: "application/pdf",
    png: "image/png",
    svg: "image/svg+xml",
    ts: "text/typescript",
    txt: "text/plain",
    webp: "image/webp",
    xlsm: "application/vnd.ms-excel.sheet.macroEnabled.12",
    xlsx: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    xml: "application/xml",
    yaml: "text/yaml",
    yml: "text/yaml",
  };
  return known[extension] ?? "application/octet-stream";
}

function spreadsheetCellValue(value: {
  type: string;
  value?: unknown;
}): string | number | boolean | null {
  if (value.type === "empty") return null;
  if (
    typeof value.value === "string" ||
    typeof value.value === "number" ||
    typeof value.value === "boolean"
  ) {
    return value.value;
  }
  if (value.value && typeof value.value === "object") {
    const serial = (value.value as { serial?: unknown }).serial;
    if (typeof serial === "number") return serial;
  }
  return value.value == null ? null : String(value.value);
}

export function parseGitStatus(output: string): GitStatusSummary {
  let branch: string | null = null;
  let upstream: string | null = null;
  let detached = false;
  let ahead = 0;
  let behind = 0;
  let changed = 0;
  let staged = 0;
  let unstaged = 0;
  let untracked = 0;

  for (const line of output.split(/\r?\n/)) {
    if (line.startsWith("# branch.head ")) {
      const value = line.slice("# branch.head ".length).trim();
      detached = value === "(detached)" || value === "(unknown)";
      branch = detached || !value ? null : value;
      continue;
    }
    if (line.startsWith("# branch.upstream ")) {
      upstream = line.slice("# branch.upstream ".length).trim() || null;
      continue;
    }
    if (line.startsWith("# branch.ab ")) {
      const match = line.match(/^# branch\.ab \+(\d+) -(\d+)$/);
      if (match) {
        ahead = Number(match[1]);
        behind = Number(match[2]);
      }
      continue;
    }
    if (line.startsWith("? ")) {
      changed += 1;
      untracked += 1;
      continue;
    }
    if (!/^[12u] /.test(line)) continue;
    const xy = line.slice(2, 4);
    if (xy.length !== 2) continue;
    changed += 1;
    if (xy[0] !== ".") staged += 1;
    if (xy[1] !== ".") unstaged += 1;
  }

  return {
    branch,
    upstream,
    detached,
    ahead,
    behind,
    changed,
    staged,
    unstaged,
    untracked,
    raw: output,
  };
}

export function parseGitBranches(output: string): GitBranchInfo[] {
  const branches: GitBranchInfo[] = [];
  for (const [index, rawLine] of output.split(/\r?\n/).entries()) {
    if (!rawLine) continue;
    const fields = rawLine.split("\0");
    if (fields.length !== 5 || !fields[0] || !fields[1]) {
      throw new Error(`无法解析第 ${index + 1} 条 Git 分支记录`);
    }
    branches.push({
      fullRef: fields[0],
      name: fields[1],
      current: fields[2] === "*",
      remote: fields[0].startsWith("refs/remotes/"),
      upstream: fields[3] || null,
      symbolicTarget: fields[4] || null,
    });
  }
  return branches.sort((left, right) => {
    if (left.current !== right.current) return left.current ? -1 : 1;
    if (left.remote !== right.remote) return left.remote ? 1 : -1;
    return left.name.localeCompare(right.name);
  });
}

function gitFailureMessage(result: GitWorkflowResponse): string {
  const detail = result.stderr.trim() || result.stdout.trim();
  const exit =
    result.exitCode === null ? "未知退出码" : `退出码 ${result.exitCode}`;
  return detail || `Git ${result.action} 执行失败（${exit}）`;
}
