import { useEffect, useMemo, useSyncExternalStore } from "react";
import type { ApiClient } from "../../api/client";
import type { FlowDefinition, FlowRun, WorkflowDeployment } from "../../types";
import { sortFlowDefinitions, sortWorkflowDeployments } from "./model";

export type WorkflowDeploymentsSnapshot = {
  status: "idle" | "loading" | "ready" | "error";
  definitions: readonly FlowDefinition[];
  deployments: readonly WorkflowDeployment[];
  selectedDeploymentId: string | null;
  editorOpen: boolean;
  busyAction: string | null;
  error: string | null;
  notice: string | null;
  lastRun: { deploymentId: string; run: FlowRun } | null;
};

const INITIAL_SNAPSHOT: WorkflowDeploymentsSnapshot = {
  status: "idle",
  definitions: [],
  deployments: [],
  selectedDeploymentId: null,
  editorOpen: false,
  busyAction: null,
  error: null,
  notice: null,
  lastRun: null,
};

export class WorkflowDeploymentsStore {
  private snapshot = INITIAL_SNAPSHOT;
  private readonly listeners = new Set<() => void>();
  private loadPromise: Promise<void> | null = null;

  constructor(private readonly client: ApiClient) {}

  readonly getSnapshot = (): WorkflowDeploymentsSnapshot => this.snapshot;

  readonly subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  async load(force = false): Promise<void> {
    if (this.loadPromise) return this.loadPromise;
    if (!force && this.snapshot.status === "ready") return;
    this.update({
      status: this.snapshot.deployments.length > 0 ? "ready" : "loading",
      error: null,
    });
    this.loadPromise = Promise.all([
      this.client.searchFlows(),
      this.client.listWorkflowDeployments(),
    ])
      .then(([definitions, deployments]) => {
        const sorted = sortWorkflowDeployments(deployments);
        const selectedDeploymentId = sorted.some(
          (deployment) => deployment.id === this.snapshot.selectedDeploymentId,
        )
          ? this.snapshot.selectedDeploymentId
          : (sorted[0]?.id ?? null);
        this.update({
          status: "ready",
          definitions: sortFlowDefinitions(definitions),
          deployments: sorted,
          selectedDeploymentId,
          error: null,
        });
      })
      .catch((error: unknown) => {
        this.update({ status: "error", error: errorMessage(error) });
      })
      .finally(() => {
        this.loadPromise = null;
      });
    return this.loadPromise;
  }

  select(deploymentId: string): void {
    if (!this.snapshot.deployments.some((item) => item.id === deploymentId)) {
      return;
    }
    this.update({
      selectedDeploymentId: deploymentId,
      editorOpen: false,
      error: null,
      notice: null,
    });
  }

  beginCreate(): void {
    this.update({ editorOpen: true, error: null, notice: null });
  }

  cancelCreate(): void {
    this.update({ editorOpen: false, error: null });
  }

  clearFeedback(): void {
    this.update({ error: null, notice: null });
  }

  async create(input: {
    flowId: string;
    flowVersion: number;
    name: string;
    environment: string;
    createdBy: string;
  }): Promise<boolean> {
    this.update({ busyAction: "create", error: null, notice: null });
    try {
      const deployment = await this.client.createWorkflowDeployment(input);
      this.update({
        deployments: sortWorkflowDeployments([
          deployment,
          ...this.snapshot.deployments.filter(
            (current) => current.id !== deployment.id,
          ),
        ]),
        selectedDeploymentId: deployment.id,
        editorOpen: false,
        notice: "Deployment Snapshot 已编译并激活",
      });
      return true;
    } catch (error) {
      this.update({ error: errorMessage(error) });
      return false;
    } finally {
      this.update({ busyAction: null });
    }
  }

  async disable(deployment: WorkflowDeployment): Promise<boolean> {
    this.update({
      busyAction: `disable:${deployment.id}`,
      error: null,
      notice: null,
    });
    try {
      const updated = await this.client.disableWorkflowDeployment(
        deployment.id,
        deployment.revision,
      );
      this.replace(updated);
      this.update({ notice: "Deployment 已停用；历史 Run 与快照保持不变" });
      return true;
    } catch (error) {
      this.update({ error: errorMessage(error) });
      return false;
    } finally {
      this.update({ busyAction: null });
    }
  }

  async run(
    threadId: string,
    deployment: WorkflowDeployment,
    input: unknown,
  ): Promise<boolean> {
    this.update({
      busyAction: `run:${deployment.id}`,
      error: null,
      notice: null,
    });
    try {
      const run = await this.client.startDeployedWorkflowRun(
        threadId,
        deployment.id,
        input,
      );
      if (this.snapshot.selectedDeploymentId === deployment.id) {
        this.update({
          lastRun: { deploymentId: deployment.id, run },
          notice: `Flow Run 已触发：${run.id}`,
        });
      }
      return true;
    } catch (error) {
      if (this.snapshot.selectedDeploymentId === deployment.id) {
        this.update({ error: errorMessage(error) });
      }
      return false;
    } finally {
      this.update({ busyAction: null });
    }
  }

  private replace(deployment: WorkflowDeployment): void {
    this.update({
      deployments: sortWorkflowDeployments([
        deployment,
        ...this.snapshot.deployments.filter(
          (current) => current.id !== deployment.id,
        ),
      ]),
    });
  }

  private update(patch: Partial<WorkflowDeploymentsSnapshot>): void {
    this.snapshot = { ...this.snapshot, ...patch };
    this.listeners.forEach((listener) => listener());
  }
}

const stores = new WeakMap<ApiClient, WorkflowDeploymentsStore>();

export function getWorkflowDeploymentsStore(
  client: ApiClient,
): WorkflowDeploymentsStore {
  const existing = stores.get(client);
  if (existing) return existing;
  const store = new WorkflowDeploymentsStore(client);
  stores.set(client, store);
  return store;
}

export function useWorkflowDeploymentsStore(client: ApiClient): {
  snapshot: WorkflowDeploymentsSnapshot;
  store: WorkflowDeploymentsStore;
} {
  const store = useMemo(() => getWorkflowDeploymentsStore(client), [client]);
  const snapshot = useSyncExternalStore(
    store.subscribe,
    store.getSnapshot,
    store.getSnapshot,
  );
  useEffect(() => {
    void store.load();
  }, [store]);
  return { snapshot, store };
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
