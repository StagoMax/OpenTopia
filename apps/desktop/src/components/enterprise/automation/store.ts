import { useEffect, useMemo, useSyncExternalStore } from "react";
import type { ApiClient } from "../../../api/client";
import type {
  WorkflowDeliveryReceipt,
  WorkflowDeployment,
  WorkflowEvaluationSummary,
  WorkflowIngressPolicy,
  WorkflowRelease,
  WorkflowTrigger,
  WorkflowTriggerInvocation,
} from "../../../types";

export type AutomationSnapshot = {
  status: "idle" | "loading" | "ready" | "error";
  releases: readonly WorkflowRelease[];
  deployments: readonly WorkflowDeployment[];
  receipts: readonly WorkflowDeliveryReceipt[];
  invocations: readonly WorkflowTriggerInvocation[];
  selectedReleaseId: string | null;
  summary: WorkflowEvaluationSummary | null;
  summaryLoading: boolean;
  createOpen: boolean;
  busyAction: string | null;
  error: string | null;
  notice: string | null;
};

const INITIAL: AutomationSnapshot = {
  status: "idle",
  releases: [],
  deployments: [],
  receipts: [],
  invocations: [],
  selectedReleaseId: null,
  summary: null,
  summaryLoading: false,
  createOpen: false,
  busyAction: null,
  error: null,
  notice: null,
};

export class WorkflowAutomationStore {
  private snapshot = INITIAL;
  private readonly listeners = new Set<() => void>();
  private loadPromise: Promise<void> | null = null;
  private summaryGeneration = 0;

  constructor(private readonly client: ApiClient) {}

  readonly getSnapshot = (): AutomationSnapshot => this.snapshot;

  readonly subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  async load(force = false): Promise<void> {
    if (this.loadPromise) return this.loadPromise;
    if (!force && this.snapshot.status === "ready") return;
    this.update({
      status: this.snapshot.releases.length ? "ready" : "loading",
      error: null,
    });
    this.loadPromise = Promise.all([
      this.client.listWorkflowReleases(),
      this.client.listWorkflowDeployments(),
      this.client.listWorkflowDeliveryReceipts(),
      this.client.listWorkflowTriggerInvocations(),
    ])
      .then(([releases, deployments, receipts, invocations]) => {
        const selectedReleaseId = releases.some(
          (release) => release.id === this.snapshot.selectedReleaseId,
        )
          ? this.snapshot.selectedReleaseId
          : (releases[0]?.id ?? null);
        this.update({
          status: "ready",
          releases: sortUpdated(releases),
          deployments: sortUpdated(deployments),
          receipts: sortUpdated(receipts),
          invocations: sortUpdated(invocations),
          selectedReleaseId,
          error: null,
        });
        void this.loadSummary();
      })
      .catch((error: unknown) => {
        this.update({ status: "error", error: errorMessage(error) });
      })
      .finally(() => {
        this.loadPromise = null;
      });
    return this.loadPromise;
  }

  select(releaseId: string): void {
    if (!this.snapshot.releases.some((release) => release.id === releaseId))
      return;
    this.update({ selectedReleaseId: releaseId, summary: null, error: null });
    void this.loadSummary();
  }

  setCreateOpen(open: boolean): void {
    this.update({ createOpen: open, error: null, notice: null });
  }

  clearFeedback(): void {
    this.update({ error: null, notice: null });
  }

  async create(input: {
    releaseKey: string;
    environment: string;
    threadId: string;
    deploymentId: string;
    trigger: WorkflowTrigger;
    ingressPolicy: WorkflowIngressPolicy;
    createdBy: string;
  }): Promise<boolean> {
    return this.perform("create", async () => {
      const release = await this.client.createWorkflowRelease(input);
      this.update({
        releases: sortUpdated([release, ...this.snapshot.releases]),
        selectedReleaseId: release.id,
        createOpen: false,
        notice: "Release Channel 已激活，外部触发 ID 保持稳定",
      });
      await this.loadSummary();
    });
  }

  async setCanary(
    release: WorkflowRelease,
    deploymentId: string,
    percent: number,
  ): Promise<boolean> {
    return this.perform(`canary:${release.id}`, async () => {
      const updated = await this.client.setWorkflowReleaseCanary(release.id, {
        expectedRevision: release.revision,
        deploymentId,
        percent,
      });
      this.replaceRelease(updated, `Canary 已设为 ${percent}%`);
    });
  }

  async promote(release: WorkflowRelease): Promise<boolean> {
    return this.perform(`promote:${release.id}`, async () => {
      const updated = await this.client.promoteWorkflowRelease(
        release.id,
        release.revision,
      );
      this.replaceRelease(
        updated,
        "Canary 已提升为 Primary；上一版本可一键回滚",
      );
      await this.loadSummary();
    });
  }

  async rollback(release: WorkflowRelease): Promise<boolean> {
    return this.perform(`rollback:${release.id}`, async () => {
      const updated = await this.client.rollbackWorkflowRelease(
        release.id,
        release.revision,
      );
      this.replaceRelease(updated, "Release 已回滚到上一 Primary Deployment");
      await this.loadSummary();
    });
  }

  async disable(release: WorkflowRelease): Promise<boolean> {
    return this.perform(`disable:${release.id}`, async () => {
      const updated = await this.client.disableWorkflowRelease(
        release.id,
        release.revision,
      );
      this.replaceRelease(
        updated,
        "Release Channel 已停用；历史运行和收据保留",
      );
    });
  }

  async retry(receipt: WorkflowDeliveryReceipt): Promise<boolean> {
    return this.perform(`retry:${receipt.id}`, async () => {
      const updated = await this.client.retryWorkflowDelivery(
        receipt.id,
        receipt.revision,
      );
      this.update({
        receipts: sortUpdated([
          updated,
          ...this.snapshot.receipts.filter((item) => item.id !== updated.id),
        ]),
        notice:
          updated.status === "delivered"
            ? "输出已重新投递"
            : "重试完成，请检查最新 DeliveryReceipt",
      });
      await this.loadSummary();
    });
  }

  async startPending(invocation: WorkflowTriggerInvocation): Promise<boolean> {
    return this.perform(`start:${invocation.id}`, async () => {
      const result = await this.client.startPendingWorkflowInvocation(
        invocation.id,
      );
      this.update({
        invocations: sortUpdated([
          result.invocation,
          ...this.snapshot.invocations.filter(
            (item) => item.id !== result.invocation.id,
          ),
        ]),
        notice: result.run
          ? `事件已批准并启动 Flow Run：${result.run.id}`
          : "事件仍在等待处理",
      });
      await this.loadSummary();
    });
  }

  private async loadSummary(): Promise<void> {
    const release = this.snapshot.releases.find(
      (item) => item.id === this.snapshot.selectedReleaseId,
    );
    if (!release) {
      this.update({ summary: null, summaryLoading: false });
      return;
    }
    const generation = ++this.summaryGeneration;
    this.update({ summaryLoading: true });
    try {
      const summary = await this.client.getWorkflowEvaluationSummary(
        release.primaryDeploymentId,
      );
      if (generation === this.summaryGeneration) this.update({ summary });
    } catch (error) {
      if (generation === this.summaryGeneration) {
        this.update({ error: errorMessage(error) });
      }
    } finally {
      if (generation === this.summaryGeneration)
        this.update({ summaryLoading: false });
    }
  }

  private replaceRelease(release: WorkflowRelease, notice: string): void {
    this.update({
      releases: sortUpdated([
        release,
        ...this.snapshot.releases.filter((item) => item.id !== release.id),
      ]),
      notice,
    });
  }

  private async perform(
    key: string,
    action: () => Promise<void>,
  ): Promise<boolean> {
    if (this.snapshot.busyAction) return false;
    this.update({ busyAction: key, error: null, notice: null });
    try {
      await action();
      return true;
    } catch (error) {
      this.update({ error: errorMessage(error) });
      return false;
    } finally {
      this.update({ busyAction: null });
    }
  }

  private update(patch: Partial<AutomationSnapshot>): void {
    this.snapshot = { ...this.snapshot, ...patch };
    this.listeners.forEach((listener) => listener());
  }
}

const stores = new WeakMap<ApiClient, WorkflowAutomationStore>();

export function useWorkflowAutomationStore(client: ApiClient): {
  snapshot: AutomationSnapshot;
  store: WorkflowAutomationStore;
} {
  const store = useMemo(() => {
    const existing = stores.get(client);
    if (existing) return existing;
    const created = new WorkflowAutomationStore(client);
    stores.set(client, created);
    return created;
  }, [client]);
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

function sortUpdated<T extends { id: string; updatedAt: string }>(
  items: readonly T[],
): T[] {
  return [...new Map(items.map((item) => [item.id, item])).values()].sort(
    (left, right) => Date.parse(right.updatedAt) - Date.parse(left.updatedAt),
  );
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
