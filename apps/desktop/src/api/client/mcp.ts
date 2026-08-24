import type {
  AgentActivityNotification,
  AgentEvent,
  McpCallResult,
  McpServerInput,
  McpServerStatus,
  McpServerView,
  McpToolDescriptor,
  TerminalEvent,
  ThreadMcpServer,
  ThreadMcpServerView,
} from "../../types";
import {
  decodeAgentActivityNotification,
  decodeAgentEvent,
  decodeTerminalEvent,
} from "../sseContracts";
import { queryString, type StreamHandle } from "./transport";
import { WorkspaceApi } from "./workspace";

export class McpApi extends WorkspaceApi {
  async listMcpServers(signal?: AbortSignal): Promise<McpServerView[]> {
    return this.get("listMcpServers", "/api/mcp/servers", signal);
  }

  async listMcpTools(serverId: string): Promise<McpToolDescriptor[]> {
    return this.get("listMcpTools", `/api/mcp/servers/${serverId}/tools`);
  }

  async createMcpServer(input: McpServerInput): Promise<McpServerView> {
    return this.post("createMcpServer", "/api/mcp/servers", input);
  }

  async updateMcpServer(
    serverId: string,
    input: McpServerInput,
  ): Promise<McpServerView> {
    return this.patch("updateMcpServer", `/api/mcp/servers/${serverId}`, {
      ...input,
      clearCwd: !input.cwd,
    });
  }

  async deleteMcpServer(serverId: string): Promise<void> {
    await this.delete("deleteMcpServer", `/api/mcp/servers/${serverId}`);
  }

  async listThreadMcpServers(
    threadId: string,
    signal?: AbortSignal,
  ): Promise<ThreadMcpServerView[]> {
    return this.get(
      "listThreadMcpServers",
      `/api/threads/${threadId}/mcp`,
      signal,
    );
  }

  async setThreadMcpServer(
    threadId: string,
    serverId: string,
    enabled: boolean,
  ): Promise<ThreadMcpServer> {
    return this.put(
      "setThreadMcpServer",
      `/api/threads/${threadId}/mcp/${serverId}`,
      { enabled },
    );
  }

  async callMcpTool(
    serverId: string,
    toolName: string,
    args: unknown,
    threadId: string,
  ): Promise<McpCallResult> {
    return this.post("callMcpTool", `/api/mcp/servers/${serverId}/call-tool`, {
      toolName,
      arguments: args,
      threadId,
    });
  }

  async restartMcpServer(serverId: string): Promise<McpServerStatus> {
    return this.post(
      "restartMcpServer",
      `/api/mcp/servers/${serverId}/restart`,
      {},
    );
  }

  openEventStream(
    threadId: string,
    since: number | undefined,
    onEvent: (event: AgentEvent) => void,
  ): StreamHandle {
    const query = queryString({ since, view: "conversation" });
    return this.openAuthenticatedSse(
      `/api/threads/${threadId}/events/stream${query}`,
      decodeAgentEvent,
      onEvent,
      "projected",
    );
  }

  openThreadActivityStream(
    onEvent: (event: AgentEvent) => void,
    onConnected?: () => void,
  ): StreamHandle {
    return this.openAuthenticatedSse(
      "/api/activity/events/stream",
      decodeAgentEvent,
      onEvent,
      "projected",
      onConnected,
    );
  }

  openAgentEventStream(
    threadId: string,
    since: number | undefined,
    onEvent: (notification: AgentActivityNotification) => void,
  ): StreamHandle {
    const query = queryString({ since });
    return this.openAuthenticatedSse(
      `/api/threads/${threadId}/agents/events/stream${query}`,
      decodeAgentActivityNotification,
      onEvent,
      "projected",
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
      decodeTerminalEvent,
      onEvent,
    );
  }
}
