export type ConversationSendTraceStage =
  | "controller_started"
  | "state_dispatched"
  | "fetch_started"
  | "response_headers"
  | "response_parsed"
  | "state_confirmed"
  | "failed";

export type ConversationSendTraceContext = {
  requestId: string;
  threadId: string;
  clientStartedAtMs: number;
  startedClockMs: number;
};

export type ConversationSendTrace = {
  stage: ConversationSendTraceStage;
  requestId: string;
  threadId: string;
  rendererAt: string;
  rendererClockMs: number;
  elapsedMs: number;
  clientStartedAtMs: number;
  turnId?: string | null;
  messageId?: string;
  queued?: boolean;
  httpStatus?: number;
  serverDurationMs?: number;
  clientToServerMs?: number;
  errorName?: string;
};

function rendererClockMs(): number {
  return typeof performance === "undefined" ? Date.now() : performance.now();
}

export function createConversationSendTraceContext(
  threadId: string,
): ConversationSendTraceContext {
  return {
    requestId: globalThis.crypto.randomUUID(),
    threadId,
    clientStartedAtMs: Date.now(),
    startedClockMs: rendererClockMs(),
  };
}

export function conversationSendTrace(
  context: ConversationSendTraceContext,
  stage: ConversationSendTraceStage,
  details: Partial<
    Pick<
      ConversationSendTrace,
      | "turnId"
      | "messageId"
      | "queued"
      | "httpStatus"
      | "serverDurationMs"
      | "clientToServerMs"
      | "errorName"
    >
  > = {},
): ConversationSendTrace {
  const clock = rendererClockMs();
  return {
    stage,
    requestId: context.requestId,
    threadId: context.threadId,
    rendererAt: new Date().toISOString(),
    rendererClockMs: clock,
    elapsedMs: Math.max(0, clock - context.startedClockMs),
    clientStartedAtMs: context.clientStartedAtMs,
    ...details,
  };
}
