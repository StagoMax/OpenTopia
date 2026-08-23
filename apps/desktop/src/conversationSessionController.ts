import type { ApiClient, StreamHandle } from "./api/client";
import {
  conversationSessionReducer,
  createConversationSessionState,
  type ConversationSessionAction,
  type ConversationSessionState,
} from "./conversationSession.ts";
import {
  inactiveTurnIdsFromEvents,
  resolveActiveTurnId,
} from "./turnActivityStatus.ts";
import type {
  AgentEvent,
  CollaborationMode,
  InlineImageAttachment,
  InlineMessageContentPart,
  LibraryProviderId,
  Message,
  UserInputResponse,
} from "./types";
import { ThreadActivityStore } from "./threadActivityStore.ts";

export type ConversationSendRequest = {
  content: string;
  sourcePaths?: string[];
  skillIds?: string[];
  collaborationMode?: CollaborationMode;
  goalId?: string;
  imageAttachments?: InlineImageAttachment[];
  contentParts?: InlineMessageContentPart[];
  libraryProvider?: LibraryProviderId;
};

export type ConversationSendResult = {
  message: Message;
  turnId: string | null;
  queued: boolean;
};

type StateListener = () => void;
type EventListener = (event: AgentEvent) => void;

export class ConversationSessionController {
  private readonly client: ApiClient;
  readonly threadId: string;
  private state: ConversationSessionState;
  private readonly stateListeners = new Set<StateListener>();
  private readonly eventListeners = new Set<EventListener>();
  private readonly eventIds = new Set<string>();
  private pendingEvents: AgentEvent[] = [];
  private eventBatchTimer: ReturnType<typeof setTimeout> | null = null;
  private stream: StreamHandle | null = null;
  private loadController: AbortController | null = null;
  private retainCount = 0;
  private loadGeneration = 0;
  private cancelRequestInFlight = false;
  private readonly activityStore?: ThreadActivityStore;

  constructor(
    client: ApiClient,
    threadId: string,
    activityStore?: ThreadActivityStore,
  ) {
    this.client = client;
    this.threadId = threadId;
    this.activityStore = activityStore;
    this.state = createConversationSessionState(threadId);
  }

  getSnapshot = (): ConversationSessionState => this.state;

  subscribe = (listener: StateListener): (() => void) => {
    this.stateListeners.add(listener);
    return () => this.stateListeners.delete(listener);
  };

  subscribeToEvents(listener: EventListener): () => void {
    this.eventListeners.add(listener);
    return () => this.eventListeners.delete(listener);
  }

  retain(): () => void {
    this.retainCount += 1;
    if (this.retainCount === 1) this.connect();
    return () => {
      this.retainCount = Math.max(0, this.retainCount - 1);
      if (this.retainCount === 0 && !this.hasLiveActivity()) {
        this.disconnect();
      }
    };
  }

  isRetained(): boolean {
    return this.retainCount > 0 || this.hasLiveActivity();
  }

  retry(): void {
    this.disconnect();
    if (this.retainCount > 0) this.connect();
  }

  async send(
    request: ConversationSendRequest,
  ): Promise<ConversationSendResult | null> {
    if (
      this.state.sending ||
      this.state.pendingApprovalIds.length > 0 ||
      this.state.pendingUserInput.length > 0
    ) {
      return null;
    }

    const startedAt = new Date().toISOString();
    this.activityStore?.startOptimistic(this.threadId);
    this.dispatch({ type: "sendStarted", startedAt });
    try {
      const result = await this.client.sendMessage(
        this.threadId,
        request.content,
        request.sourcePaths ?? [],
        request.skillIds ?? [],
        request.collaborationMode ?? "default",
        request.goalId,
        request.imageAttachments ?? [],
        request.contentParts ?? [],
        request.libraryProvider,
      );
      this.dispatch({
        type: "sendSucceeded",
        ...result,
        startedAt,
      });
      this.activityStore?.confirmTurn(this.threadId, result.turnId);
      if (this.state.cancellationRequested && result.turnId) {
        await this.issueCancel(result.turnId);
      }
      return result;
    } catch (error) {
      this.activityStore?.clearOptimistic(this.threadId);
      this.dispatch({
        type: "sendFailed",
        error: errorMessage(error),
        startedAt,
      });
      return null;
    }
  }

  async cancel(): Promise<void> {
    if (this.state.cancelling) return;
    if (
      !this.state.activeTurnId &&
      !this.state.sending &&
      !this.state.pendingTurnFeedback
    ) {
      return;
    }
    this.dispatch({ type: "cancelRequested" });
    await this.issueCancel(this.state.activeTurnId ?? undefined);
  }

  async decideApproval(
    approvalId: string,
    approved: boolean,
  ): Promise<boolean> {
    if (this.state.decidingApprovalId) return false;
    this.dispatch({ type: "approvalStarted", approvalId });
    try {
      const decision = await this.client.decideApproval(
        this.threadId,
        approvalId,
        approved,
      );
      if (!decision.accepted) {
        throw new Error("服务端未接受该审批决定，请重试。");
      }
      this.dispatch({ type: "approvalSucceeded", approvalId });
      return true;
    } catch (error) {
      this.dispatch({
        type: "approvalFailed",
        error: `审批决定提交失败：${errorMessage(error)}`,
      });
      return false;
    }
  }

  async respondToUserInput(
    requestId: string,
    response: UserInputResponse,
  ): Promise<boolean> {
    if (this.state.submittingUserInputId) return false;
    this.dispatch({ type: "userInputStarted", requestId });
    try {
      const result = await this.client.respondToUserInput(
        this.threadId,
        requestId,
        response,
      );
      if (!result.accepted || (!response.cancelled && !result.resumed)) {
        throw new Error("服务端未恢复当前任务，请重试。");
      }
      this.dispatch({ type: "userInputSucceeded", requestId });
      return true;
    } catch (error) {
      this.dispatch({
        type: "userInputFailed",
        error: `无法提交选择：${errorMessage(error)}`,
      });
      return false;
    }
  }

  appendLocalEvent(event: AgentEvent): void {
    this.dispatch({ type: "eventsReceived", events: [event] });
  }

  replaceEvents(update: (events: AgentEvent[]) => AgentEvent[]): void {
    this.dispatch({
      type: "eventsReplaced",
      events: update(this.state.events),
    });
  }

  clearCommandError(): void {
    this.dispatch({ type: "clearCommandError" });
  }

  dispose(): void {
    this.retainCount = 0;
    this.disconnect();
    this.stateListeners.clear();
    this.eventListeners.clear();
  }

  private connect(): void {
    if (this.loadController || this.stream) return;
    const generation = ++this.loadGeneration;
    const controller = new AbortController();
    this.loadController = controller;
    this.dispatch({ type: "loadStarted" });

    const since = latestPersistedEventSeq(this.state.events);
    void Promise.all([
      this.client.listMessages(this.threadId, controller.signal),
      this.client.listConversationEvents(
        this.threadId,
        since,
        controller.signal,
      ),
    ])
      .then(([messages, events]) => {
        if (!this.isCurrentLoad(generation, controller)) return;
        events.forEach((event) => this.rememberEventId(event.id));
        this.activityStore?.applyEvents(events);
        this.dispatch({ type: "historyLoaded", messages, events });
        this.stream = this.client.openEventStream(
          this.threadId,
          latestPersistedEventSeq(this.state.events),
          (event) => this.receiveEvent(event),
        );
      })
      .catch((error) => {
        if (
          !this.isCurrentLoad(generation, controller) ||
          isAbortError(error)
        ) {
          return;
        }
        this.dispatch({ type: "loadFailed", error: errorMessage(error) });
      });

    void Promise.allSettled([
      this.client.getTurnStatus(this.threadId, controller.signal),
      this.client.listPendingApprovals(this.threadId, controller.signal),
      this.client.listPendingUserInput(this.threadId, controller.signal),
    ]).then(([turnStatus, approvals, userInput]) => {
      if (!this.isCurrentLoad(generation, controller)) return;
      if (turnStatus.status === "fulfilled") {
        this.activityStore?.reconcileTurnStatus(turnStatus.value);
      }
      this.dispatch({
        type: "auxiliaryLoaded",
        turnStatus:
          turnStatus.status === "fulfilled" ? turnStatus.value : undefined,
        pendingApprovalIds:
          approvals.status === "fulfilled"
            ? approvals.value.map((approval) => approval.approvalId)
            : undefined,
        pendingUserInput:
          userInput.status === "fulfilled" ? userInput.value : undefined,
      });
    });
  }

  private disconnect(): void {
    this.loadGeneration += 1;
    this.loadController?.abort();
    this.loadController = null;
    this.stream?.close();
    this.stream = null;
    this.flushPendingEvents();
  }

  private hasLiveActivity(): boolean {
    return (
      this.activityStore?.isLive(this.threadId) === true ||
      this.state.sending ||
      this.state.pendingTurnFeedback !== null ||
      this.state.activeTurnId !== null ||
      this.state.turnStatus?.status === "running" ||
      this.state.turnStatus?.status === "cancelling"
    );
  }

  private isCurrentLoad(
    generation: number,
    controller: AbortController,
  ): boolean {
    return (
      generation === this.loadGeneration &&
      this.loadController === controller &&
      !controller.signal.aborted
    );
  }

  private receiveEvent(event: AgentEvent): void {
    if (event.threadId !== this.threadId || this.eventIds.has(event.id)) return;
    this.rememberEventId(event.id);
    this.activityStore?.applyEvent(event);
    this.eventListeners.forEach((listener) => listener(event));
    this.pendingEvents.push(event);
    if (!this.eventBatchTimer) {
      this.eventBatchTimer = setTimeout(() => this.flushPendingEvents(), 32);
    }
    if (
      event.payload.type === "turn_started" &&
      event.turnId &&
      this.state.cancellationRequested
    ) {
      void this.issueCancel(event.turnId);
    }
  }

  private flushPendingEvents(): void {
    if (this.eventBatchTimer) clearTimeout(this.eventBatchTimer);
    this.eventBatchTimer = null;
    if (this.pendingEvents.length === 0) return;
    const events = this.pendingEvents;
    this.pendingEvents = [];
    this.dispatch({ type: "eventsReceived", events });
  }

  private rememberEventId(eventId: string): void {
    this.eventIds.add(eventId);
    if (this.eventIds.size <= 4096) return;
    const oldestId = this.eventIds.values().next().value;
    if (oldestId) this.eventIds.delete(oldestId);
  }

  private async issueCancel(turnId?: string): Promise<void> {
    if (this.cancelRequestInFlight) return;
    this.cancelRequestInFlight = true;
    try {
      const result = await this.client.cancelTurn(this.threadId, turnId);
      if (result.cancelled) return;
      let activeTurnId = turnId ?? null;
      try {
        const turnStatus = await this.client.getTurnStatus(this.threadId);
        activeTurnId = resolveActiveTurnId(
          turnStatus,
          inactiveTurnIdsFromEvents(this.state.events),
        );
      } catch {
        // Preserve the cancellation response when reconciliation is unavailable.
      }
      this.dispatch({
        type: "cancelReconciled",
        activeTurnId,
        error: activeTurnId && turnId ? result.message : undefined,
      });
    } catch (error) {
      this.dispatch({
        type: "cancelFailed",
        error: `中断执行失败：${errorMessage(error)}`,
      });
    } finally {
      this.cancelRequestInFlight = false;
    }
  }

  private dispatch(action: ConversationSessionAction): void {
    const next = conversationSessionReducer(this.state, action);
    if (next === this.state) return;
    this.state = next;
    this.stateListeners.forEach((listener) => listener());
    if (this.retainCount === 0 && !this.hasLiveActivity() && this.stream) {
      this.disconnect();
    }
  }
}

export class ConversationSessionRegistry {
  private readonly client: ApiClient;
  private readonly cacheLimit: number;
  private readonly eventListeners = new Set<EventListener>();
  readonly activityStore: ThreadActivityStore;
  private readonly controllers = new Map<
    string,
    ConversationSessionController
  >();
  private readonly controllerEventReleases = new Map<string, () => void>();
  private readonly activityRetentionReleases = new Map<string, () => void>();
  private readonly activityStoreRelease: () => void;

  constructor(
    client: ApiClient,
    cacheLimit = 8,
    activityStore = new ThreadActivityStore(),
  ) {
    this.client = client;
    this.cacheLimit = cacheLimit;
    this.activityStore = activityStore;
    this.activityStoreRelease = activityStore.subscribeToChanges((threadId) =>
      this.syncActivityRetention(threadId),
    );
    activityStore
      .liveThreadIds()
      .forEach((threadId) => this.syncActivityRetention(threadId));
  }

  get(threadId: string): ConversationSessionController {
    const existing = this.controllers.get(threadId);
    if (existing) {
      this.controllers.delete(threadId);
      this.controllers.set(threadId, existing);
      return existing;
    }
    const controller = new ConversationSessionController(
      this.client,
      threadId,
      this.activityStore,
    );
    const releaseEvents = controller.subscribeToEvents((event) => {
      this.eventListeners.forEach((listener) => listener(event));
    });
    this.controllers.set(threadId, controller);
    this.controllerEventReleases.set(threadId, releaseEvents);
    this.prune(threadId);
    return controller;
  }

  subscribeToEvents(listener: EventListener): () => void {
    this.eventListeners.add(listener);
    return () => this.eventListeners.delete(listener);
  }

  dispose(): void {
    this.activityStoreRelease();
    this.activityRetentionReleases.forEach((release) => release());
    this.activityRetentionReleases.clear();
    this.controllerEventReleases.forEach((release) => release());
    this.controllerEventReleases.clear();
    this.controllers.forEach((controller) => controller.dispose());
    this.controllers.clear();
    this.eventListeners.clear();
  }

  private syncActivityRetention(threadId: string): void {
    const retained = this.activityRetentionReleases.get(threadId);
    if (this.activityStore.isLive(threadId)) {
      if (retained) return;
      const release = this.get(threadId).retain();
      this.activityRetentionReleases.set(threadId, release);
      return;
    }
    if (!retained) return;
    this.activityRetentionReleases.delete(threadId);
    retained();
  }

  private prune(protectedThreadId: string): void {
    if (this.controllers.size <= this.cacheLimit) return;
    for (const [threadId, controller] of this.controllers) {
      if (this.controllers.size <= this.cacheLimit) break;
      if (threadId === protectedThreadId || controller.isRetained()) continue;
      this.controllerEventReleases.get(threadId)?.();
      this.controllerEventReleases.delete(threadId);
      controller.dispose();
      this.controllers.delete(threadId);
    }
  }
}

function latestPersistedEventSeq(events: AgentEvent[]): number | undefined {
  let latest: number | undefined;
  for (const event of events) {
    if (event.seq === Number.MAX_SAFE_INTEGER) continue;
    latest = latest === undefined ? event.seq : Math.max(latest, event.seq);
  }
  return latest;
}

function isAbortError(error: unknown): boolean {
  return error instanceof Error && error.name === "AbortError";
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
