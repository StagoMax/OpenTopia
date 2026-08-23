import type { AgentEvent, ToolCall, ToolResult } from "./types";

export type ToolFailureDetail = {
  eventId: string;
  callId: string;
  turnId: string | null;
  sequence: number;
  createdAt: string;
  call: ToolCall | null;
  toolName: string;
  code: string | null;
  phase: string | null;
  executed: boolean | null;
  retryable: boolean | null;
  message: string;
  causes: string[];
};

export function toolResultIsError(result?: ToolResult): boolean {
  if (!result) return false;
  const metadata = asRecord(result.metadata);
  if (!metadata) return false;
  return (
    metadata.success === false ||
    metadata.isError === true ||
    hasOwn(metadata, "toolError") ||
    hasOwn(metadata, "errorRecord") ||
    hasOwn(metadata, "error")
  );
}

export function collectToolFailures(
  events: readonly AgentEvent[],
): ToolFailureDetail[] {
  const calls = new Map<string, ToolCall>();
  for (const event of events) {
    if (event.payload.type !== "tool_call_started") continue;
    calls.set(
      callKey(event.threadId, event.payload.call.id),
      event.payload.call,
    );
  }

  return events
    .flatMap((event): ToolFailureDetail[] => {
      if (
        event.payload.type !== "tool_call_finished" ||
        !toolResultIsError(event.payload.result)
      ) {
        return [];
      }

      const result = event.payload.result;
      const metadata = asRecord(result.metadata);
      const errorRecord = asRecord(metadata?.errorRecord);
      const toolError = asRecord(metadata?.toolError);
      const call = calls.get(callKey(event.threadId, result.callId)) ?? null;
      const message = firstText(
        errorRecord?.message,
        metadata?.error,
        typeof metadata?.toolError === "string" ? metadata.toolError : null,
        toolError?.message,
        toolError?.error,
        result.output,
      );

      return [
        {
          eventId: event.id,
          callId: result.callId,
          turnId: event.turnId ?? null,
          sequence: event.seq,
          createdAt: event.createdAt,
          call,
          toolName: call?.name || firstText(metadata?.toolName) || "未知工具",
          code: optionalText(errorRecord?.code),
          phase: optionalText(errorRecord?.phase),
          executed: optionalBoolean(errorRecord?.executed),
          retryable: optionalBoolean(errorRecord?.retryable),
          message: message || "工具调用失败，但没有记录具体原因。",
          causes: collectCauses(errorRecord, metadata, message),
        },
      ];
    })
    .sort((left, right) => right.sequence - left.sequence);
}

function collectCauses(
  errorRecord: Record<string, unknown> | null,
  metadata: Record<string, unknown> | null,
  message: string,
): string[] {
  const candidates = [errorRecord?.causes, metadata?.errorChain];
  const causes = candidates.flatMap((candidate) =>
    Array.isArray(candidate)
      ? candidate.filter(
          (value): value is string =>
            typeof value === "string" && value.trim().length > 0,
        )
      : [],
  );
  return [...new Set(causes.map((cause) => cause.trim()))].filter(
    (cause) => cause !== message,
  );
}

function callKey(threadId: string, callId: string): string {
  return `${threadId}\u0000${callId}`;
}

function hasOwn(record: Record<string, unknown>, key: string): boolean {
  return Object.prototype.hasOwnProperty.call(record, key);
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function firstText(...values: unknown[]): string {
  for (const value of values) {
    if (typeof value === "string" && value.trim().length > 0) {
      return value.trim();
    }
  }
  return "";
}

function optionalText(value: unknown): string | null {
  const text = firstText(value);
  return text || null;
}

function optionalBoolean(value: unknown): boolean | null {
  return typeof value === "boolean" ? value : null;
}
