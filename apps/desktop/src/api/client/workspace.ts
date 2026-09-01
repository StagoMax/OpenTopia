import type {
  ArtifactContent,
  ArtifactDescriptor,
  ContextStatus,
  ContextSummary,
  DiffFileActionResult,
  GitBranchInfo,
  GitStatusSummary,
  GitWorkflowAction,
  GitWorkflowResponse,
  PreviewDescriptor,
  PreviewTarget,
  SandboxDescriptor,
  SpreadsheetPreview,
  SpreadsheetPreviewRange,
  TerminalCancelResponse,
  TerminalEvent,
  TerminalSession,
  TerminalStartResponse,
  TurnChangeSet,
  TurnFileDiffPreview,
  TurnUndoPreview,
  TurnUndoResult,
  UserInputRecord,
  UserInputResponse,
  WorkspaceDiff,
  WorkspaceDiffHunk,
  WorkspaceDiffHunkAction,
  WorkspaceFilePreview,
  WorkspaceTree,
} from "../../types";
import { ConversationApi } from "./conversation";
import { ApiResponseError, queryString } from "./transport";
import {
  gitFailureMessage,
  mapPreviewDescriptor,
  parseGitBranches,
  parseGitStatus,
  spreadsheetCellValue,
  type PreviewDescriptorResponse,
  type SpreadsheetRangeResponse,
  type SpreadsheetWorkbookResponse,
} from "./workspaceHelpers";

export class WorkspaceApi extends ConversationApi {
  async startTerminalCommand(
    threadId: string,
    command: string,
    options?: { cwd?: string; timeoutMs?: number },
  ): Promise<TerminalStartResponse> {
    return this.post(
      "startTerminalCommand",
      `/api/threads/${threadId}/terminal/commands`,
      {
        command,
        cwd: options?.cwd,
        timeoutMs: options?.timeoutMs,
      },
    );
  }

  async cancelTerminalCommand(
    threadId: string,
    commandId?: string,
  ): Promise<TerminalCancelResponse> {
    return this.post(
      "cancelTerminalCommand",
      `/api/threads/${threadId}/terminal/cancel`,
      {
        commandId,
      },
    );
  }

  async listTerminalHistory(
    threadId: string,
    since?: number,
    signal?: AbortSignal,
  ): Promise<TerminalEvent[]> {
    return this.get(
      "listTerminalHistory",
      `/api/threads/${threadId}/terminal/history${queryString({ since })}`,
      signal,
    );
  }

  async getTerminalSession(threadId: string): Promise<TerminalSession | null> {
    return this.get(
      "getTerminalSession",
      `/api/threads/${threadId}/terminal/session`,
    );
  }

  async ensureTerminalSession(
    threadId: string,
    options?: { cwd?: string; cols?: number; rows?: number },
  ): Promise<TerminalSession> {
    return this.post(
      "ensureTerminalSession",
      `/api/threads/${threadId}/terminal/session`,
      options ?? {},
    );
  }

  async writeTerminalSession(
    threadId: string,
    sessionId: string,
    data: string,
  ): Promise<TerminalSession> {
    return this.post(
      "writeTerminalSession",
      `/api/threads/${threadId}/terminal/session/input`,
      {
        sessionId,
        data,
      },
    );
  }

  async resizeTerminalSession(
    threadId: string,
    sessionId: string,
    cols: number,
    rows: number,
  ): Promise<TerminalSession> {
    return this.post(
      "resizeTerminalSession",
      `/api/threads/${threadId}/terminal/session/resize`,
      {
        sessionId,
        cols,
        rows,
      },
    );
  }

  async closeTerminalSession(
    threadId: string,
    sessionId: string,
  ): Promise<TerminalSession> {
    return this.post(
      "closeTerminalSession",
      `/api/threads/${threadId}/terminal/session/close`,
      {
        sessionId,
      },
    );
  }

  async decideApproval(
    threadId: string,
    approvalId: string,
    approved: boolean,
  ): Promise<{ accepted: boolean; executed: boolean }> {
    return this.post(
      "decideApproval",
      `/api/threads/${threadId}/approvals/${approvalId}/decision`,
      { approved },
    );
  }

  async listPendingApprovals(
    threadId: string,
    signal?: AbortSignal,
  ): Promise<Array<{ approvalId: string }>> {
    return this.get(
      "listPendingApprovals",
      `/api/threads/${threadId}/approvals?status=pending`,
      signal,
    );
  }

  async listPendingUserInput(
    threadId: string,
    signal?: AbortSignal,
  ): Promise<UserInputRecord[]> {
    return this.get(
      "listPendingUserInput",
      `/api/threads/${threadId}/user-input?status=pending`,
      signal,
    );
  }

  async respondToUserInput(
    threadId: string,
    requestId: string,
    response: UserInputResponse,
  ): Promise<{ accepted: boolean; resumed: boolean }> {
    return this.post(
      "respondToUserInput",
      `/api/threads/${threadId}/user-input/${requestId}/response`,
      response,
    );
  }

  async listWorkspaceTree(
    threadId: string,
    path?: string,
    signal?: AbortSignal,
  ): Promise<WorkspaceTree> {
    return this.get(
      "listWorkspaceTree",
      `/api/threads/${threadId}/workspace/tree${queryString({ path })}`,
      signal,
    );
  }

  async readWorkspaceFile(
    threadId: string,
    path: string,
  ): Promise<WorkspaceFilePreview> {
    return this.get(
      "readWorkspaceFile",
      `/api/threads/${threadId}/workspace/file${queryString({ path })}`,
    );
  }

  async getWorkspaceDiff(
    threadId: string,
    signal?: AbortSignal,
  ): Promise<WorkspaceDiff> {
    return this.get(
      "getWorkspaceDiff",
      `/api/threads/${threadId}/workspace/diff`,
      signal,
    );
  }

  async getTurnChanges(
    threadId: string,
    turnId: string,
  ): Promise<TurnChangeSet> {
    return this.get(
      "getTurnChanges",
      `/api/threads/${threadId}/turns/${turnId}/changes`,
    );
  }

  async getTurnFileDiffPreview(
    threadId: string,
    turnId: string,
    path: string,
    offset = 0,
  ): Promise<TurnFileDiffPreview> {
    return this.get(
      "getTurnFileDiffPreview",
      `/api/threads/${threadId}/turns/${turnId}/changes/preview${queryString({
        path,
        offset,
      })}`,
    );
  }

  async previewTurnUndo(
    threadId: string,
    turnId: string,
  ): Promise<TurnUndoPreview> {
    return this.post(
      "previewTurnUndo",
      `/api/threads/${threadId}/turns/${turnId}/undo/preview`,
      {},
    );
  }

  async undoTurnChanges(
    threadId: string,
    turnId: string,
  ): Promise<TurnUndoResult> {
    return this.post(
      "undoTurnChanges",
      `/api/threads/${threadId}/turns/${turnId}/undo`,
      {
        confirm: true,
      },
    );
  }

  async runGitWorkflow(
    threadId: string,
    action: GitWorkflowAction,
  ): Promise<GitWorkflowResponse> {
    const result = await this.post<GitWorkflowResponse>(
      "runGitWorkflow",
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
    return this.post(
      "revertWorkspaceFile",
      `/api/threads/${threadId}/workspace/diff/revert`,
      {
        path,
        confirm,
      },
    );
  }

  async applyWorkspaceDiffHunk(
    threadId: string,
    hunk: WorkspaceDiffHunk,
    action: WorkspaceDiffHunkAction,
    confirm: boolean,
  ): Promise<DiffFileActionResult> {
    return this.post(
      "applyWorkspaceDiffHunk",
      `/api/threads/${threadId}/workspace/diff/hunk`,
      {
        path: hunk.path,
        scope: hunk.scope,
        patch: hunk.patch ?? hunk.raw,
        action,
        confirm,
      },
    );
  }

  async getSandbox(
    threadId: string,
    signal?: AbortSignal,
  ): Promise<SandboxDescriptor> {
    return this.get("getSandbox", `/api/threads/${threadId}/sandbox`, signal);
  }

  async getContextStatus(
    threadId: string,
    signal?: AbortSignal,
  ): Promise<ContextStatus> {
    return this.get(
      "getContextStatus",
      `/api/threads/${threadId}/context`,
      signal,
    );
  }

  async compactContext(
    threadId: string,
    summary?: string,
  ): Promise<ContextSummary> {
    return this.post(
      "compactContext",
      `/api/threads/${threadId}/context/compact`,
      { summary },
    );
  }

  async listArtifacts(
    threadId: string,
    signal?: AbortSignal,
  ): Promise<ArtifactDescriptor[]> {
    return this.get(
      "listArtifacts",
      `/api/threads/${threadId}/artifacts`,
      signal,
    );
  }

  async getArtifact(
    threadId: string,
    artifactId: string,
  ): Promise<ArtifactContent> {
    const artifact = await this.get<{
      id: string;
      storage:
        { type: "inline"; content: string } | { type: "path"; path: string };
    }>("getArtifact", `/api/threads/${threadId}/artifacts/${artifactId}`);
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
        capabilities: {
          read: true,
          write: false,
          watch: false,
          rangeRead: false,
          openExternal: false,
        },
      };
    }

    const response = await this.post<PreviewDescriptorResponse>(
      "resolvePreview",
      `/api/threads/${threadId}/resources/resolve`,
      target.type === "workspace"
        ? { source: "workspace", path: target.path }
        : target.type === "local"
          ? { source: "local", path: target.path }
          : target.type === "artifact"
            ? { source: "artifact", artifactId: target.artifactId }
            : { source: "attachment", attachmentId: target.attachmentId },
    );
    return mapPreviewDescriptor(response, threadId, target);
  }

  async getPreviewContent(threadId: string, previewId: string): Promise<Blob> {
    const response = await fetch(
      `${this.baseUrl}/api/threads/${threadId}/resources/${encodeURIComponent(previewId)}/content`,
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

  async getResourceMetadata(
    descriptor: PreviewDescriptor,
  ): Promise<PreviewDescriptor> {
    const response = await this.get<PreviewDescriptorResponse>(
      "getResourceMetadata",
      `/api/threads/${descriptor.threadId}/resources/${encodeURIComponent(descriptor.id)}`,
    );
    return mapPreviewDescriptor(
      response,
      descriptor.threadId,
      descriptor.target,
    );
  }

  async writeResourceContent(
    descriptor: PreviewDescriptor,
    content: string,
    expectedRevision: string,
  ): Promise<PreviewDescriptor> {
    const response = await this.put<PreviewDescriptorResponse>(
      "writeResourceContent",
      `/api/threads/${descriptor.threadId}/resources/${encodeURIComponent(descriptor.id)}/content`,
      { content, expectedRevision },
    );
    return mapPreviewDescriptor(
      response,
      descriptor.threadId,
      descriptor.target,
    );
  }

  async getSpreadsheetPreview(
    threadId: string,
    previewId: string,
  ): Promise<SpreadsheetPreview> {
    const workbook = await this.get<SpreadsheetWorkbookResponse>(
      "getSpreadsheetPreview",
      `/api/threads/${threadId}/resources/${encodeURIComponent(previewId)}/workbook`,
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
    signal?: AbortSignal,
  ): Promise<SpreadsheetPreviewRange> {
    const response = await this.get<SpreadsheetRangeResponse>(
      "getSpreadsheetPreviewRange",
      `/api/threads/${threadId}/resources/${encodeURIComponent(previewId)}/range${queryString(
        {
          sheet: sheetId,
          startRow: input.rowStart,
          rowCount: input.rowCount,
          startColumn: input.columnStart,
          columnCount: input.columnCount,
        },
      )}`,
      signal,
    );
    const cells = response.rows.flatMap((row, rowOffset) =>
      row.map((cell, columnOffset) => ({
        row: response.range.start.row + rowOffset,
        column: response.range.start.column + columnOffset,
        value: spreadsheetCellValue(cell.value),
        formula: cell.formula,
        formatted: cell.formatted,
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

  async closePreview(threadId: string, previewId: string): Promise<void> {
    if (!previewId.startsWith("resource.")) return;
    await this.delete(
      "closePreview",
      `/api/threads/${threadId}/resources/${encodeURIComponent(previewId)}`,
    );
  }
}
