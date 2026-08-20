import { useEffect, useMemo, useSyncExternalStore } from "react";
import type { ApiClient } from "../../api/client";
import type {
  Connection,
  ConnectionCapabilityRevision,
  ConnectionInput,
  ConnectionTestResult,
  IntegrationDefinition,
  McpServerView,
} from "../../types";
import { connectionUpdateFromInput, sortConnections } from "./model";

export type ConnectionEditorMode = "create" | "edit" | null;

export type ConnectionsSnapshot = {
  status: "idle" | "loading" | "ready" | "error";
  definitions: readonly IntegrationDefinition[];
  connections: readonly Connection[];
  mcpServers: readonly McpServerView[];
  selectedConnectionId: string | null;
  editorMode: ConnectionEditorMode;
  capabilityRevisions: Readonly<
    Record<string, readonly ConnectionCapabilityRevision[]>
  >;
  busyAction: string | null;
  error: string | null;
  notice: string | null;
  lastHealth: {
    connectionId: string;
    health: ConnectionTestResult["health"];
  } | null;
};

const INITIAL_SNAPSHOT: ConnectionsSnapshot = {
  status: "idle",
  definitions: [],
  connections: [],
  mcpServers: [],
  selectedConnectionId: null,
  editorMode: null,
  capabilityRevisions: {},
  busyAction: null,
  error: null,
  notice: null,
  lastHealth: null,
};

export class ConnectionsStore {
  private snapshot = INITIAL_SNAPSHOT;
  private readonly listeners = new Set<() => void>();
  private loadPromise: Promise<void> | null = null;
  private readonly capabilityLoads = new Map<string, Promise<void>>();

  constructor(private readonly client: ApiClient) {}

  readonly getSnapshot = (): ConnectionsSnapshot => this.snapshot;

  readonly subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  async load(force = false): Promise<void> {
    if (this.loadPromise) return this.loadPromise;
    if (!force && this.snapshot.status === "ready") return;
    this.update({
      status: this.snapshot.connections.length > 0 ? "ready" : "loading",
      error: null,
    });
    this.loadPromise = Promise.all([
      this.client.listIntegrationDefinitions(),
      this.client.listConnections(),
      this.client.listMcpServers(),
    ])
      .then(([definitions, connections, mcpServers]) => {
        const sorted = sortConnections(connections);
        const selectedConnectionId = sorted.some(
          (connection) => connection.id === this.snapshot.selectedConnectionId,
        )
          ? this.snapshot.selectedConnectionId
          : (sorted[0]?.id ?? null);
        this.update({
          status: "ready",
          definitions,
          connections: sorted,
          mcpServers,
          selectedConnectionId,
          error: null,
        });
        if (selectedConnectionId)
          void this.loadCapabilities(selectedConnectionId);
      })
      .catch((error: unknown) => {
        this.update({ status: "error", error: errorMessage(error) });
      })
      .finally(() => {
        this.loadPromise = null;
      });
    return this.loadPromise;
  }

  select(connectionId: string): void {
    if (!this.snapshot.connections.some((item) => item.id === connectionId)) {
      return;
    }
    this.update({
      selectedConnectionId: connectionId,
      editorMode: null,
      error: null,
      notice: null,
      lastHealth: null,
    });
    void this.loadCapabilities(connectionId);
  }

  beginCreate(): void {
    this.update({
      editorMode: "create",
      error: null,
      notice: null,
      lastHealth: null,
    });
  }

  beginEdit(): void {
    if (!this.snapshot.selectedConnectionId) return;
    this.update({ editorMode: "edit", error: null, notice: null });
  }

  cancelEdit(): void {
    this.update({ editorMode: null, error: null });
  }

  clearFeedback(): void {
    this.update({ error: null, notice: null });
  }

  async save(input: ConnectionInput): Promise<boolean> {
    const editing =
      this.snapshot.editorMode === "edit"
        ? this.selectedConnection()
        : undefined;
    const action = editing ? `save:${editing.id}` : "create";
    this.update({ busyAction: action, error: null, notice: null });
    try {
      const saved = editing
        ? await this.client.updateConnection(
            editing.id,
            connectionUpdateFromInput(editing, input),
          )
        : await this.client.createConnection(input);
      this.replaceConnection(saved);
      this.update({
        selectedConnectionId: saved.id,
        editorMode: null,
        notice: editing ? "Connection 配置已更新" : "Connection 已创建",
        lastHealth: null,
      });
      await this.loadCapabilities(saved.id, true);
      return true;
    } catch (error) {
      this.update({ error: errorMessage(error) });
      return false;
    } finally {
      this.update({ busyAction: null });
    }
  }

  async test(connectionId = this.snapshot.selectedConnectionId): Promise<void> {
    if (!connectionId) return;
    this.update({
      busyAction: `test:${connectionId}`,
      error: null,
      notice: null,
      lastHealth: null,
    });
    try {
      const result = await this.client.testConnection(connectionId);
      this.replaceConnection(result.connection);
      if (this.snapshot.selectedConnectionId === connectionId) {
        this.update({
          lastHealth: { connectionId, health: result.health },
          notice: result.health.ok ? "连接测试通过" : null,
          error: result.health.ok ? null : result.health.message,
        });
      }
    } catch (error) {
      if (this.snapshot.selectedConnectionId === connectionId) {
        this.update({ error: errorMessage(error) });
      }
    } finally {
      this.update({ busyAction: null });
    }
  }

  async refreshCapabilities(
    connectionId = this.snapshot.selectedConnectionId,
  ): Promise<void> {
    if (!connectionId) return;
    this.update({
      busyAction: `refresh:${connectionId}`,
      error: null,
      notice: null,
    });
    try {
      const result =
        await this.client.refreshConnectionCapabilities(connectionId);
      this.replaceConnection(result.connection);
      const current = this.snapshot.capabilityRevisions[connectionId] ?? [];
      const revisions = [
        result.capabilityRevision,
        ...current.filter(
          (revision) => revision.id !== result.capabilityRevision.id,
        ),
      ];
      const feedback =
        this.snapshot.selectedConnectionId === connectionId
          ? {
              notice: result.changed
                ? `能力快照已更新：新增 ${result.diff.addedCapabilityIds.length}、移除 ${result.diff.removedCapabilityIds.length}、变更 ${result.diff.changedCapabilityIds.length}`
                : `能力未变化，仍为 ${result.capabilityRevision.capabilities.length} 项`,
            }
          : {};
      this.update({
        capabilityRevisions: {
          ...this.snapshot.capabilityRevisions,
          [connectionId]: revisions,
        },
        ...feedback,
      });
    } catch (error) {
      if (this.snapshot.selectedConnectionId === connectionId) {
        this.update({ error: errorMessage(error) });
      }
    } finally {
      this.update({ busyAction: null });
    }
  }

  private async loadCapabilities(
    connectionId: string,
    force = false,
  ): Promise<void> {
    if (!force && this.snapshot.capabilityRevisions[connectionId]) return;
    const existing = this.capabilityLoads.get(connectionId);
    if (existing) return existing;
    const request = this.client
      .listConnectionCapabilityRevisions(connectionId)
      .then((revisions) => {
        this.update({
          capabilityRevisions: {
            ...this.snapshot.capabilityRevisions,
            [connectionId]: [...revisions].sort(
              (left, right) => right.revision - left.revision,
            ),
          },
        });
      })
      .catch((error: unknown) => {
        if (this.snapshot.selectedConnectionId === connectionId) {
          this.update({ error: errorMessage(error) });
        }
      })
      .finally(() => {
        this.capabilityLoads.delete(connectionId);
      });
    this.capabilityLoads.set(connectionId, request);
    return request;
  }

  private selectedConnection(): Connection | undefined {
    return this.snapshot.connections.find(
      (connection) => connection.id === this.snapshot.selectedConnectionId,
    );
  }

  private replaceConnection(connection: Connection): void {
    this.update({
      connections: sortConnections([
        connection,
        ...this.snapshot.connections.filter(
          (item) => item.id !== connection.id,
        ),
      ]),
    });
  }

  private update(patch: Partial<ConnectionsSnapshot>): void {
    this.snapshot = { ...this.snapshot, ...patch };
    for (const listener of this.listeners) listener();
  }
}

const stores = new WeakMap<ApiClient, ConnectionsStore>();

export function getConnectionsStore(client: ApiClient): ConnectionsStore {
  const existing = stores.get(client);
  if (existing) return existing;
  const created = new ConnectionsStore(client);
  stores.set(client, created);
  return created;
}

export function useConnectionsStore(client: ApiClient): {
  store: ConnectionsStore;
  snapshot: ConnectionsSnapshot;
} {
  const store = useMemo(() => getConnectionsStore(client), [client]);
  const snapshot = useSyncExternalStore(
    store.subscribe,
    store.getSnapshot,
    store.getSnapshot,
  );
  useEffect(() => {
    void store.load();
  }, [store]);
  return { store, snapshot };
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
