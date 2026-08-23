import { useEffect, useMemo, useSyncExternalStore } from "react";
import type { ApiClient } from "../../api/client";
import type {
  AgentInstance,
  AgentTemplateVersionView,
  Connection,
  FlowDefinition,
  FlowRun,
  HumanTask,
  WorkflowDeployment,
  WorkflowTriggerInvocation,
} from "../../types";

export type EnterpriseSnapshot = {
  status: "idle" | "loading" | "ready" | "error";
  templates: readonly AgentTemplateVersionView[];
  agents: readonly AgentInstance[];
  workflows: readonly FlowDefinition[];
  deployments: readonly WorkflowDeployment[];
  runs: readonly FlowRun[];
  tasks: readonly HumanTask[];
  invocations: readonly WorkflowTriggerInvocation[];
  connections: readonly Connection[];
  error: string | null;
  refreshedAt: string | null;
};

const INITIAL_SNAPSHOT: EnterpriseSnapshot = {
  status: "idle",
  templates: [],
  agents: [],
  workflows: [],
  deployments: [],
  runs: [],
  tasks: [],
  invocations: [],
  connections: [],
  error: null,
  refreshedAt: null,
};

export class EnterpriseStore {
  private snapshot = INITIAL_SNAPSHOT;
  private readonly listeners = new Set<() => void>();
  private loadPromise: Promise<void> | null = null;

  constructor(private readonly client: ApiClient) {}

  readonly getSnapshot = (): EnterpriseSnapshot => this.snapshot;

  readonly subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  async load(force = false): Promise<void> {
    if (this.loadPromise) return this.loadPromise;
    if (!force && this.snapshot.status === "ready") return;
    this.update({
      status: this.snapshot.refreshedAt ? "ready" : "loading",
      error: null,
    });
    this.loadPromise = Promise.all([
      this.client.listAgentTemplates(),
      this.client.listAgentInstances({ limit: 200 }),
      this.client.searchFlows(),
      this.client.listWorkflowDeployments(),
      this.client.listAllFlowRuns({ limit: 200 }),
      this.client.listHumanTasks({ status: "pending" }),
      this.client.listWorkflowTriggerInvocations(),
      this.client.listConnections(),
    ])
      .then(
        ([
          templates,
          agents,
          workflows,
          deployments,
          runs,
          tasks,
          invocations,
          connections,
        ]) => {
          this.update({
            status: "ready",
            templates,
            agents,
            workflows,
            deployments,
            runs,
            tasks,
            invocations,
            connections,
            error: null,
            refreshedAt: new Date().toISOString(),
          });
        },
      )
      .catch((error: unknown) => {
        this.update({ status: "error", error: errorMessage(error) });
      })
      .finally(() => {
        this.loadPromise = null;
      });
    return this.loadPromise;
  }

  private update(patch: Partial<EnterpriseSnapshot>): void {
    this.snapshot = { ...this.snapshot, ...patch };
    for (const listener of this.listeners) listener();
  }
}

const stores = new WeakMap<ApiClient, EnterpriseStore>();

export function getEnterpriseStore(client: ApiClient): EnterpriseStore {
  const existing = stores.get(client);
  if (existing) return existing;
  const created = new EnterpriseStore(client);
  stores.set(client, created);
  return created;
}

export function useEnterpriseStore(client: ApiClient): {
  store: EnterpriseStore;
  snapshot: EnterpriseSnapshot;
} {
  const store = useMemo(() => getEnterpriseStore(client), [client]);
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
