import type {
  Connection,
  ConnectionCapabilityRefreshResult,
  ConnectionCapabilityRevision,
  ConnectionInput,
  ConnectionStatus,
  ConnectionTestResult,
  ConnectionUpdate,
  IntegrationDefinition,
  IntegrationDefinitionInput,
  IntegrationDefinitionUpdate,
} from "../../types";
import { McpApi } from "./mcp";
import { queryString } from "./transport";

export class ConnectionsApi extends McpApi {
  async listIntegrationDefinitions(
    signal?: AbortSignal,
  ): Promise<IntegrationDefinition[]> {
    return this.get(
      "listIntegrationDefinitions",
      "/api/integration-definitions",
      signal,
    );
  }

  async getIntegrationDefinition(
    definitionId: string,
    signal?: AbortSignal,
  ): Promise<IntegrationDefinition> {
    return this.get(
      "getIntegrationDefinition",
      `/api/integration-definitions/${definitionId}`,
      signal,
    );
  }

  async createIntegrationDefinition(
    input: IntegrationDefinitionInput,
  ): Promise<IntegrationDefinition> {
    return this.post(
      "createIntegrationDefinition",
      "/api/integration-definitions",
      input,
    );
  }

  async updateIntegrationDefinition(
    definitionId: string,
    input: IntegrationDefinitionUpdate,
  ): Promise<IntegrationDefinition> {
    return this.patch(
      "updateIntegrationDefinition",
      `/api/integration-definitions/${definitionId}`,
      input,
    );
  }

  async listConnections(
    filters: {
      integrationDefinitionId?: string;
      status?: ConnectionStatus;
    } = {},
    signal?: AbortSignal,
  ): Promise<Connection[]> {
    const query = queryString(filters);
    return this.get("listConnections", `/api/connections${query}`, signal);
  }

  async getConnection(
    connectionId: string,
    signal?: AbortSignal,
  ): Promise<Connection> {
    return this.get(
      "getConnection",
      `/api/connections/${connectionId}`,
      signal,
    );
  }

  async createConnection(input: ConnectionInput): Promise<Connection> {
    return this.post("createConnection", "/api/connections", input);
  }

  async updateConnection(
    connectionId: string,
    input: ConnectionUpdate,
  ): Promise<Connection> {
    return this.patch(
      "updateConnection",
      `/api/connections/${connectionId}`,
      input,
    );
  }

  async testConnection(connectionId: string): Promise<ConnectionTestResult> {
    return this.post(
      "testConnection",
      `/api/connections/${connectionId}/test`,
      {},
    );
  }

  async refreshConnectionCapabilities(
    connectionId: string,
  ): Promise<ConnectionCapabilityRefreshResult> {
    return this.post(
      "refreshConnectionCapabilities",
      `/api/connections/${connectionId}/capabilities/refresh`,
      {},
    );
  }

  async listConnectionCapabilityRevisions(
    connectionId: string,
    signal?: AbortSignal,
  ): Promise<ConnectionCapabilityRevision[]> {
    return this.get(
      "listConnectionCapabilityRevisions",
      `/api/connections/${connectionId}/capability-revisions`,
      signal,
    );
  }

  async getConnectionCapabilityRevision(
    connectionId: string,
    revision: number,
    signal?: AbortSignal,
  ): Promise<ConnectionCapabilityRevision> {
    return this.get(
      "getConnectionCapabilityRevision",
      `/api/connections/${connectionId}/capability-revisions/${revision}`,
      signal,
    );
  }
}
