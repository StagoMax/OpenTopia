import {
  isThreadActivityUnread,
  readThreadActivityReadAt,
  writeThreadActivityReadAt,
  type ThreadActivityReadAt,
} from "./threadActivityRead.ts";
import {
  resolveThreadActivityEventStatus,
  resolveThreadActivityStatus,
  type ThreadActivityStatus,
} from "./threadActivityStatus.ts";
import type { AgentEvent, TurnStatus } from "./types";

export type ThreadActivityPhase = ThreadActivityStatus | "cancelled";

export type ThreadActivityRecord = Readonly<{
  threadId: string;
  turnId: string | null;
  phase: ThreadActivityPhase;
  lastEventSeq: number | null;
  updatedAt: string;
  unread: boolean;
  optimistic: boolean;
}>;

type Listener = () => void;
type ChangeListener = (
  threadId: string,
  record: ThreadActivityRecord | null,
) => void;

type ThreadActivityStoreOptions = {
  readAt?: ThreadActivityReadAt;
  now?: () => string;
  persistReadAt?: (readAt: ThreadActivityReadAt) => void;
};

const livePhases = new Set<ThreadActivityPhase>([
  "processing",
  "approval",
  "user_action",
]);

export class ThreadActivityStore {
  private readonly records = new Map<string, ThreadActivityRecord>();
  private readonly listeners = new Set<Listener>();
  private readonly threadListeners = new Map<string, Set<Listener>>();
  private readonly changeListeners = new Set<ChangeListener>();
  private readonly now: () => string;
  private readonly persistReadAt: (readAt: ThreadActivityReadAt) => void;
  private readAt: ThreadActivityReadAt;
  private visibleStatuses: Readonly<Record<string, ThreadActivityStatus>> = {};
  private batchDepth = 0;
  private readonly pendingChanges = new Map<
    string,
    ThreadActivityRecord | null
  >();
  private readonly pendingVisibleThreads = new Set<string>();
  private visibleSnapshotChanged = false;

  constructor(options: ThreadActivityStoreOptions = {}) {
    this.readAt = options.readAt ?? readThreadActivityReadAt();
    this.now = options.now ?? (() => new Date().toISOString());
    this.persistReadAt = options.persistReadAt ?? writeThreadActivityReadAt;
  }

  getVisibleStatusesSnapshot = (): Readonly<
    Record<string, ThreadActivityStatus>
  > => this.visibleStatuses;

  getVisibleStatus(threadId: string | null): ThreadActivityStatus | undefined {
    return threadId ? this.visibleStatuses[threadId] : undefined;
  }

  getRecord(threadId: string): ThreadActivityRecord | null {
    return this.records.get(threadId) ?? null;
  }

  isLive(threadId: string): boolean {
    const record = this.records.get(threadId);
    return Boolean(record && livePhases.has(record.phase));
  }

  liveThreadIds(): string[] {
    return [...this.records.values()]
      .filter((record) => livePhases.has(record.phase))
      .map((record) => record.threadId);
  }

  subscribe = (listener: Listener): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  subscribeThread(threadId: string | null, listener: Listener): () => void {
    if (!threadId) return () => {};
    const listeners = this.threadListeners.get(threadId) ?? new Set<Listener>();
    listeners.add(listener);
    this.threadListeners.set(threadId, listeners);
    return () => {
      listeners.delete(listener);
      if (listeners.size === 0) this.threadListeners.delete(threadId);
    };
  }

  subscribeToChanges(listener: ChangeListener): () => void {
    this.changeListeners.add(listener);
    return () => this.changeListeners.delete(listener);
  }

  startOptimistic(threadId: string): void {
    const current = this.records.get(threadId);
    this.setRecord({
      threadId,
      turnId: null,
      phase: "processing",
      lastEventSeq: current?.lastEventSeq ?? null,
      updatedAt: this.now(),
      unread: false,
      optimistic: true,
    });
  }

  confirmTurn(threadId: string, turnId: string | null): void {
    if (!turnId) return;
    const current = this.records.get(threadId);
    if (!current || !livePhases.has(current.phase)) return;
    if (current.turnId && current.turnId !== turnId) return;
    this.setRecord({ ...current, turnId, optimistic: false });
  }

  clearOptimistic(threadId: string): void {
    const current = this.records.get(threadId);
    if (!current?.optimistic) return;
    this.deleteRecord(threadId);
  }

  markRead(threadId: string): void {
    const readAt = this.now();
    this.readAt = { ...this.readAt, [threadId]: readAt };
    this.persistReadAt(this.readAt);

    const current = this.records.get(threadId);
    if (!current?.unread) return;
    this.setRecord({ ...current, unread: false });
  }

  applyEvent(event: AgentEvent): void {
    const projectedStatus = resolveThreadActivityEventStatus(event);
    if (projectedStatus === undefined) return;

    const current = this.records.get(event.threadId);
    if (this.isStaleEvent(current, event)) return;
    if (current?.optimistic && isBefore(event.createdAt, current.updatedAt)) {
      return;
    }

    const phase: ThreadActivityPhase =
      projectedStatus === null ? "cancelled" : projectedStatus;
    const incomingTurnId = event.turnId ?? null;
    const incomingIsLive = livePhases.has(phase);
    const startsTurn = event.payload.type === "turn_started";

    if (current && livePhases.has(current.phase)) {
      if (
        current.turnId &&
        incomingTurnId !== current.turnId &&
        !startsTurn &&
        !(phase === "failed" && incomingTurnId === null)
      ) {
        return;
      }
      if (
        current.optimistic &&
        !incomingIsLive &&
        incomingTurnId !== current.turnId
      ) {
        return;
      }
      if (!current.turnId && !incomingTurnId && phase !== "failed") return;
    }

    const eventSeq =
      persistedEventSeq(event.seq) ?? current?.lastEventSeq ?? null;
    const unread = incomingIsLive
      ? false
      : isThreadActivityUnread(this.readAt, event.threadId, event.createdAt);
    this.setRecord({
      threadId: event.threadId,
      turnId: incomingTurnId ?? current?.turnId ?? null,
      phase,
      lastEventSeq: eventSeq,
      updatedAt: event.createdAt,
      unread,
      optimistic: false,
    });
  }

  applyEvents(events: readonly AgentEvent[]): void {
    this.batch(() => {
      [...events]
        .sort((left, right) => left.seq - right.seq)
        .forEach((event) => this.applyEvent(event));
    });
  }

  reconcileTurnStatus(turnStatus: TurnStatus | null): void {
    if (!turnStatus) return;
    const current = this.records.get(turnStatus.threadId);
    const projectedStatus = resolveThreadActivityStatus(turnStatus);
    const phase: ThreadActivityPhase =
      projectedStatus ??
      (turnStatus.status === "cancelled" ? "cancelled" : "failed");
    const incomingIsLive = livePhases.has(phase);

    if (current && livePhases.has(current.phase)) {
      if (
        current.turnId &&
        current.turnId !== turnStatus.turnId &&
        (!incomingIsLive || !isAfter(turnStatus.startedAt, current.updatedAt))
      ) {
        return;
      }
      if (
        current.optimistic &&
        current.turnId !== turnStatus.turnId &&
        (!incomingIsLive || isBefore(turnStatus.startedAt, current.updatedAt))
      ) {
        return;
      }
    } else if (
      current &&
      current.turnId !== turnStatus.turnId &&
      !isAfter(turnStatus.updatedAt, current.updatedAt)
    ) {
      return;
    }

    this.setRecord({
      threadId: turnStatus.threadId,
      turnId: turnStatus.turnId,
      phase,
      lastEventSeq: current?.lastEventSeq ?? null,
      updatedAt: turnStatus.updatedAt,
      unread: incomingIsLive
        ? false
        : isThreadActivityUnread(
            this.readAt,
            turnStatus.threadId,
            turnStatus.updatedAt,
          ),
      optimistic: false,
    });
  }

  reconcileTurnStatuses(turnStatuses: readonly TurnStatus[]): void {
    this.batch(() => {
      turnStatuses.forEach((turnStatus) =>
        this.reconcileTurnStatus(turnStatus),
      );
    });
  }

  retainKnownThreads(threadIds: ReadonlySet<string>): void {
    [...this.records.keys()].forEach((threadId) => {
      if (!threadIds.has(threadId)) this.deleteRecord(threadId);
    });
  }

  reset(): void {
    if (this.records.size === 0) return;
    const threadIds = [...this.records.keys()];
    this.records.clear();
    this.visibleStatuses = {};
    this.listeners.forEach((listener) => listener());
    threadIds.forEach((threadId) => {
      this.threadListeners.get(threadId)?.forEach((listener) => listener());
      this.changeListeners.forEach((listener) => listener(threadId, null));
    });
  }

  private isStaleEvent(
    current: ThreadActivityRecord | undefined,
    event: AgentEvent,
  ): boolean {
    const eventSeq = persistedEventSeq(event.seq);
    return Boolean(
      current?.lastEventSeq !== null &&
      current?.lastEventSeq !== undefined &&
      eventSeq !== null &&
      eventSeq <= current.lastEventSeq,
    );
  }

  private setRecord(next: ThreadActivityRecord): void {
    const current = this.records.get(next.threadId);
    if (current && recordsEqual(current, next)) return;
    const previousVisibleStatus = this.visibleStatuses[next.threadId];
    const nextVisibleStatus = visibleStatus(next);
    this.records.set(next.threadId, next);
    this.emitRecordChange(next.threadId, next);
    if (previousVisibleStatus === nextVisibleStatus) return;

    const visibleStatuses = { ...this.visibleStatuses };
    if (nextVisibleStatus) visibleStatuses[next.threadId] = nextVisibleStatus;
    else delete visibleStatuses[next.threadId];
    this.visibleStatuses = visibleStatuses;
    this.emitVisibleChange(next.threadId);
  }

  private deleteRecord(threadId: string): void {
    if (!this.records.delete(threadId)) return;
    const wasVisible = threadId in this.visibleStatuses;
    if (wasVisible) {
      const visibleStatuses = { ...this.visibleStatuses };
      delete visibleStatuses[threadId];
      this.visibleStatuses = visibleStatuses;
      this.emitVisibleChange(threadId);
    }
    this.emitRecordChange(threadId, null);
  }

  private batch(update: () => void): void {
    this.batchDepth += 1;
    try {
      update();
    } finally {
      this.batchDepth -= 1;
      if (this.batchDepth === 0) this.flushPendingNotifications();
    }
  }

  private emitRecordChange(
    threadId: string,
    record: ThreadActivityRecord | null,
  ): void {
    if (this.batchDepth > 0) {
      this.pendingChanges.set(threadId, record);
      return;
    }
    this.changeListeners.forEach((listener) => listener(threadId, record));
  }

  private emitVisibleChange(threadId: string): void {
    if (this.batchDepth > 0) {
      this.visibleSnapshotChanged = true;
      this.pendingVisibleThreads.add(threadId);
      return;
    }
    this.listeners.forEach((listener) => listener());
    this.threadListeners.get(threadId)?.forEach((listener) => listener());
  }

  private flushPendingNotifications(): void {
    this.pendingChanges.forEach((record, threadId) => {
      this.changeListeners.forEach((listener) => listener(threadId, record));
    });
    this.pendingChanges.clear();
    if (this.visibleSnapshotChanged) {
      this.listeners.forEach((listener) => listener());
      this.visibleSnapshotChanged = false;
    }
    this.pendingVisibleThreads.forEach((threadId) => {
      this.threadListeners.get(threadId)?.forEach((listener) => listener());
    });
    this.pendingVisibleThreads.clear();
  }
}

function visibleStatus(
  record: ThreadActivityRecord,
): ThreadActivityStatus | undefined {
  if (record.phase === "cancelled") return undefined;
  if (livePhases.has(record.phase)) return record.phase;
  return record.unread ? record.phase : undefined;
}

function persistedEventSeq(seq: number): number | null {
  return seq === Number.MAX_SAFE_INTEGER ? null : seq;
}

function recordsEqual(
  left: ThreadActivityRecord,
  right: ThreadActivityRecord,
): boolean {
  return (
    left.threadId === right.threadId &&
    left.turnId === right.turnId &&
    left.phase === right.phase &&
    left.lastEventSeq === right.lastEventSeq &&
    left.updatedAt === right.updatedAt &&
    left.unread === right.unread &&
    left.optimistic === right.optimistic
  );
}

function isBefore(left: string, right: string): boolean {
  const leftMs = Date.parse(left);
  const rightMs = Date.parse(right);
  return (
    Number.isFinite(leftMs) && Number.isFinite(rightMs) && leftMs < rightMs
  );
}

function isAfter(left: string, right: string): boolean {
  const leftMs = Date.parse(left);
  const rightMs = Date.parse(right);
  return (
    Number.isFinite(leftMs) && Number.isFinite(rightMs) && leftMs > rightMs
  );
}
