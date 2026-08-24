import type { ApiClient, StreamHandle } from "./api/client";
import {
  conversationSessionReducer,
  createConversationSessionState,
  type ConversationSessionAction,
  type ConversationSessionState,
} from "./conversationSession.ts";
import {
  conversationSendTrace,
  createConversationSendTraceContext,
} from "./conversationSendTrace.ts";
import { recordConversationSendTrace } from "./platform.ts";
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
import { ConversationHistoryLoader } from "./conversationHistoryLoader.ts";

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
  private olderLoadController: AbortController | null = null;
  private catchUpController: AbortController | null = null;
  private retainCount = 0;
  private loadGeneration = 0;
  private cancelRequestInFlight = false;
  private readonly activityStore: ThreadActivityStore;
  private readonly historyLoader: ConversationHistoryLoader;

  constructor(
    client: ApiClient,
    threadId: string,
    activityStore = new ThreadActivityStore(),
  ) {
    this.client = client;
    this.threadId = threadId;
    this.activityStore = activityStore;
    this.historyLoader = new ConversationHistoryLoader(client, threadId);
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
      if (this.retainCount === 0) this.disconnect();
    };
  }

  isRetained(): boolean {
    return (
      this.retainCount > 0 ||
      this.activityStore.getRunState(this.threadId).sending
    );
  }

  retry(): void {
    this.disconnect();
    if (this.retainCount > 0) this.connect();
  }

  async loadOlderMessages(): Promise<void> {
    if (
      this.olderLoadController ||
      this.state.loadingOlderMessages ||
      !this.state.hasOlderMessages
    ) {
      return;
    }
    const firstMessage = this.state.messages[0];
    if (!firstMessage) return;

    const controller = new AbortController();
    this.olderLoadController = controller;
    this.dispatch({ type: "olderMessagesLoadStarted" });
    try {
      const page = await this.historyLoader.loadOlderMessages(
        firstMessage,
        controller.signal,
      );
      if (controller.signal.aborted) return;
      const events = await this.historyLoader.loadEventsForOlderMessages(
        page.messages,
        this.state.events[0]?.seq,
        controller.signal,
      );
      if (controller.signal.aborted) return;
      events.forEach((event) => this.rememberEventId(event.id));
      this.dispatch({
        type: "olderMessagesLoaded",
        messages: page.messages,
        events,
        hasOlderMessages: page.hasOlderMessages,
      });
    } catch (error) {
      if (!isAbortError(error)) {
        this.dispatch({
          type: "olderMessagesLoadFailed",
          error: errorMessage(error),
        });
      }
    } finally {
      if (this.olderLoadController === controller) {
        this.olderLoadController = null;
      }
    }
  }

  async send(
    request: ConversationSendRequest,
  ): Promise<ConversationSendResult | null> {
    if (
      this.activityStore.getRunState(this.threadId).sending ||
      this.state.pendingApprovalIds.length > 0 ||
      this.state.pendingUserInput.length > 0
    ) {
      return null;
    }

    const trace = createConversationSendTraceContext(this.threadId);
    recordConversationSendTrace(
      conversationSendTrace(trace, "controller_started"),
    );
    const startedAt = new Date().toISOString();
    this.activityStore.beginSend(this.threadId);
    this.dispatch({ type: "commandStarted" });
    recordConversationSendTrace(
      conversationSendTrace(trace, "state_dispatched"),
    );
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
        trace,
      );
      this.dispatch({
        type: "sendSucceeded",
        message: result.message,
        queued: result.queued,
      });
      recordConversationSendTrace(
        conversationSendTrace(trace, "state_confirmed", {
          turnId: result.turnId,
          messageId: result.message.id,
          queued: result.queued,
        }),
      );
      this.activityStore.confirmSend(this.threadId, {
        threadId: this.threadId,
        turnId: result.turnId,
        userMessageId: result.message.id,
        startedAt,
      });
      if (
        this.activityStore.getRunState(this.threadId).cancellationRequested &&
        result.turnId
      ) {
        await this.issueCancel(result.turnId);
      }
      return result;
    } catch (error) {
      recordConversationSendTrace(
        conversationSendTrace(trace, "failed", {
          errorName: error instanceof Error ? error.name : typeof error,
        }),
      );
      this.activityStore.failSend(this.threadId);
      this.dispatch({
        type: "sendFailed",
        error: errorMessage(error),
      });
      return null;
    }
  }

  async cancel(): Promise<void> {
    const runState = this.activityStore.getRunState(this.threadId);
    if (runState.cancelling) return;
    if (
      !runState.activeTurnId &&
      !runState.sending &&
      !runState.pendingTurnFeedback
    ) {
      return;
    }
    this.activityStore.requestCancellation(this.threadId);
    this.dispatch({ type: "commandStarted" });
    await this.issueCancel(runState.activeTurnId ?? undefined);
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

  receiveActivityEvent(event: AgentEvent): void {
    if (this.retainCount === 0 || event.threadId !== this.threadId) return;
    const since = latestPersistedEventSeq(this.state.events);
    this.receiveEvent(event);
    if (!isTerminalActivityEvent(event)) return;
    this.catchUpController?.abort();
    const controller = new AbortController();
    this.catchUpController = controller;
    void this.historyLoader
      .loadForwardEvents(since, controller.signal)
      .then((events) => {
        if (controller.signal.aborted || this.retainCount === 0) return;
        events.forEach((nextEvent) => this.receiveEvent(nextEvent));
      })
      .catch((error) => {
        if (isAbortError(error)) return;
        console.warn("OpenTopia conversation catch-up failed", error);
      })
      .finally(() => {
        if (this.catchUpController === controller) {
          this.catchUpController = null;
        }
      });
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
    const hasSnapshot = this.state.loadState.status === "ready";
    const generation = ++this.loadGeneration;
    const controller = new AbortController();
    this.loadController = controller;
    this.dispatch({ type: "loadStarted" });

    const since = latestPersistedEventSeq(this.state.events);
    const loadingFeedbackPainted = waitForLoadingFeedbackPaint();
    void (async () => {
      try {
        const messagesPromise = hasSnapshot
          ? this.historyLoader.loadMessageDelta(
              this.state.messages,
              this.state.hasOlderMessages,
              controller.signal,
            )
          : this.historyLoader.loadInitialMessages(controller.signal);
        const eventsPromise = hasSnapshot
          ? this.historyLoader.loadForwardEvents(since, controller.signal)
          : null;
        const auxiliaryPromise = Promise.allSettled([
          this.client.getTurnStatus(this.threadId, controller.signal),
          this.client.listPendingApprovals(this.threadId, controller.signal),
          this.client.listPendingUserInput(this.threadId, controller.signal),
        ]);

        const messagePage = await messagesPromise;
        const events = eventsPromise
          ? await eventsPromise
          : await this.historyLoader.loadInitialEvents(
              messagePage.messages,
              controller.signal,
            );
        const [turnStatus, approvals, userInput] = await auxiliaryPromise;
        // Keep the data requests parallel, but do not replace the switching
        // state before the renderer has had one opportunity to paint it.
        await loadingFeedbackPainted;
        if (!this.isCurrentLoad(generation, controller)) return;
        events.forEach((event) => this.rememberEventId(event.id));
        this.activityStore.applyEvents(events);
        if (turnStatus.status === "fulfilled") {
          this.activityStore.reconcileTurnStatus(turnStatus.value);
        }
        // Cached content remains visible behind the refresh affordance, but
        // messages, events and decision state become visible in one commit.
        this.dispatch({
          type: "syncCompleted",
          messages: messagePage.messages,
          events,
          hasOlderMessages: messagePage.hasOlderMessages,
          pendingApprovalIds:
            approvals.status === "fulfilled"
              ? approvals.value.map((approval) => approval.approvalId)
              : undefined,
          pendingUserInput:
            userInput.status === "fulfilled" ? userInput.value : undefined,
        });
        if (
          !this.isCurrentLoad(generation, controller) ||
          this.retainCount === 0
        ) {
          return;
        }
        this.stream = this.client.openEventStream(
          this.threadId,
          latestPersistedEventSeq(this.state.events),
          (event) => this.receiveEvent(event),
        );
      } catch (error) {
        if (
          !this.isCurrentLoad(generation, controller) ||
          isAbortError(error)
        ) {
          return;
        }
        this.dispatch({ type: "loadFailed", error: errorMessage(error) });
      }
    })();
  }

  private disconnect(): void {
    this.loadGeneration += 1;
    this.loadController?.abort();
    this.loadController = null;
    this.olderLoadController?.abort();
    this.olderLoadController = null;
    this.catchUpController?.abort();
    this.catchUpController = null;
    this.stream?.close();
    this.stream = null;
    this.flushPendingEvents();
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
    this.activityStore.applyEvent(event);
    this.eventListeners.forEach((listener) => listener(event));
    this.pendingEvents.push(event);
    if (!this.eventBatchTimer) {
      this.eventBatchTimer = setTimeout(() => this.flushPendingEvents(), 32);
    }
    if (
      event.payload.type === "turn_started" &&
      event.turnId &&
      this.activityStore.getRunState(this.threadId).cancellationRequested
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
      let turnStatusResolved = false;
      try {
        const turnStatus = await this.client.getTurnStatus(this.threadId);
        this.activityStore.reconcileTurnStatus(turnStatus);
        activeTurnId = this.activityStore.getRunState(
          this.threadId,
        ).activeTurnId;
        turnStatusResolved = turnStatus !== null;
      } catch {
        // Preserve the cancellation response when reconciliation is unavailable.
      }
      this.activityStore.reconcileCancellation(
        this.threadId,
        activeTurnId,
        turnStatusResolved,
      );
      this.dispatch({
        type: "cancelReconciled",
        error: activeTurnId && turnId ? result.message : undefined,
      });
    } catch (error) {
      this.activityStore.failCancellation(this.threadId);
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
    if (this.retainCount === 0 && this.stream) this.disconnect();
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
  private readonly activityStream: StreamHandle;
  private activityReconcileGeneration = 0;

  constructor(
    client: ApiClient,
    cacheLimit = 8,
    activityStore = new ThreadActivityStore(),
  ) {
    this.client = client;
    this.cacheLimit = cacheLimit;
    this.activityStore = activityStore;
    this.activityStream = client.openThreadActivityStream(
      (event) => {
        this.activityStore.applyEvent(event);
        this.controllers.get(event.threadId)?.receiveActivityEvent(event);
      },
      () => this.reconcileLiveActivity(),
    );
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
    this.activityReconcileGeneration += 1;
    this.activityStream.close();
    this.controllerEventReleases.forEach((release) => release());
    this.controllerEventReleases.clear();
    this.controllers.forEach((controller) => controller.dispose());
    this.controllers.clear();
    this.eventListeners.clear();
  }

  private reconcileLiveActivity(): void {
    const generation = ++this.activityReconcileGeneration;
    const baseline = this.activityStore.captureLiveReconciliationBaseline();
    void this.client
      .listActivityStatuses()
      .then((statuses) => {
        if (generation !== this.activityReconcileGeneration) return;
        this.activityStore.reconcileLiveTurnStatuses(statuses, baseline);
      })
      .catch(() => undefined);
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

function waitForLoadingFeedbackPaint(): Promise<void> {
  if (typeof requestAnimationFrame !== "function") {
    return Promise.resolve();
  }
  return new Promise((resolve) => {
    requestAnimationFrame(() => setTimeout(resolve, 0));
  });
}

function isTerminalActivityEvent(event: AgentEvent): boolean {
  return (
    event.payload.type === "turn_finished" ||
    event.payload.type === "turn_cancelled" ||
    event.payload.type === "error"
  );
}

function isAbortError(error: unknown): boolean {
  return error instanceof Error && error.name === "AbortError";
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
