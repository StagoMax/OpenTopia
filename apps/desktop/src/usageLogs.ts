import type { AgentEvent, ThreadModelSelection } from "./types";

export type UsageCallStatus = "running" | "succeeded" | "failed";

export type UsageCall = {
  id: string;
  turnId: string | null;
  round: number;
  providerId: string | null;
  model: string | null;
  adapter: string;
  endpoint: string;
  startedAt: string;
  completedAt: string | null;
  durationMs: number | null;
  ttftMs: number | null;
  statusCode: number | null;
  status: UsageCallStatus;
  attemptCount: number;
  retryCount: number;
  contextTokenEstimate: number | null;
  inputTokens: number;
  cachedInputTokens: number;
  cacheReadTokensReported: boolean;
  cacheWriteTokens: number;
  cacheWriteTokensReported: boolean;
  outputTokens: number;
  reasoningTokens: number;
  totalTokens: number;
};

export type UsageSummary = {
  requestCount: number;
  successfulRequestCount: number;
  failedRequestCount: number;
  runningRequestCount: number;
  turnCount: number;
  inputTokens: number;
  cachedInputTokens: number;
  cacheReadInputTokens: number;
  cacheReadReportedRequestCount: number;
  cacheWriteTokens: number;
  cacheWriteReportedRequestCount: number;
  outputTokens: number;
  reasoningTokens: number;
  totalTokens: number;
  cacheReadRatio: number | null;
  averageTokensPerRequest: number | null;
  averageLatencyMs: number | null;
  p95LatencyMs: number | null;
  averageTtftMs: number | null;
  outputTokensPerSecond: number | null;
  retryCount: number;
  retryRate: number | null;
  errorEventCount: number;
  toolCallCount: number;
  toolErrorCount: number;
  averageToolDurationMs: number | null;
};

export type UsageDashboardData = {
  calls: UsageCall[];
  summary: UsageSummary;
};

type MutableUsageCall = UsageCall & {
  firstTokenAt: string | null;
  responseReceived: boolean;
  usageReceived: boolean;
  errored: boolean;
};

type ProviderContext = {
  providerId: string | null;
  model: string | null;
};

type AggregateUsageOptions = {
  fallbackModelSelection?: ThreadModelSelection | null;
};

const terminalTurnEvents = new Set([
  "turn_finished",
  "turn_cancelled",
  "turn_suspended",
  "turn_awaiting_input",
  "error",
]);

export function aggregateUsageEvents(
  sourceEvents: AgentEvent[],
  options: AggregateUsageOptions = {},
): UsageDashboardData {
  const events = [...sourceEvents].sort((left, right) => left.seq - right.seq);
  let providerContext: ProviderContext = {
    providerId: options.fallbackModelSelection?.connectionId ?? null,
    model: options.fallbackModelSelection?.modelId ?? null,
  };
  const contextEstimateByRequest = new Map<string, number>();
  const callsById = new Map<string, MutableUsageCall>();
  const latestRequestByTurn = new Map<string, string>();
  const includedTurns = new Set<string>();
  const terminalTurns = new Set<string>();
  const toolStarts = new Map<string, number>();
  const toolDurations: number[] = [];
  let errorEventCount = 0;
  let toolCallCount = 0;
  let toolErrorCount = 0;

  for (const event of events) {
    const payload = event.payload;

    if (payload.type === "thread_context_snapshot") {
      providerContext = {
        providerId: payload.snapshot.providerId,
        model: payload.snapshot.model,
      };
    } else if (payload.type === "provider_context_state_updated") {
      providerContext = {
        providerId: payload.provider_id,
        model: payload.model,
      };
    }

    if (payload.type === "model_context_built") {
      contextEstimateByRequest.set(payload.request_id, payload.token_estimate);
    }

    const turnKey = event.turnId ?? "__thread";

    if (payload.type === "turn_started" && event.turnId) {
      includedTurns.add(event.turnId);
    }

    if (terminalTurnEvents.has(payload.type) && event.turnId) {
      terminalTurns.add(event.turnId);
      includedTurns.add(event.turnId);
    }

    switch (payload.type) {
      case "provider_request_sent": {
        if (event.turnId) includedTurns.add(event.turnId);
        const call: MutableUsageCall = {
          id: payload.request_id,
          turnId: event.turnId ?? null,
          round: payload.round,
          providerId: providerContext.providerId,
          model: providerContext.model,
          adapter: payload.adapter,
          endpoint: payload.endpoint,
          startedAt: event.createdAt,
          completedAt: null,
          durationMs: null,
          ttftMs: null,
          statusCode: null,
          status: "running",
          attemptCount: Math.max(1, payload.attempt),
          retryCount: 0,
          contextTokenEstimate:
            contextEstimateByRequest.get(payload.request_id) ?? null,
          inputTokens: 0,
          cachedInputTokens: 0,
          cacheReadTokensReported: false,
          cacheWriteTokens: 0,
          cacheWriteTokensReported: false,
          outputTokens: 0,
          reasoningTokens: 0,
          totalTokens: 0,
          firstTokenAt: null,
          responseReceived: false,
          usageReceived: false,
          errored: false,
        };
        callsById.set(call.id, call);
        latestRequestByTurn.set(turnKey, call.id);
        break;
      }
      case "provider_request_retried": {
        const call = callsById.get(payload.request_id);
        if (!call) break;
        call.attemptCount = Math.max(call.attemptCount, payload.attempt);
        call.retryCount += 1;
        break;
      }
      case "provider_response_received": {
        const call = callsById.get(payload.request_id);
        if (!call) break;
        call.attemptCount = Math.max(call.attemptCount, payload.attempt);
        call.statusCode = payload.status ?? null;
        call.completedAt = event.createdAt;
        call.durationMs = elapsedMs(call.startedAt, event.createdAt);
        call.responseReceived = true;
        break;
      }
      case "model_delta": {
        const call = latestCallForTurn(callsById, latestRequestByTurn, turnKey);
        if (!call || call.firstTokenAt) break;
        call.firstTokenAt = event.createdAt;
        call.ttftMs = elapsedMs(call.startedAt, event.createdAt);
        break;
      }
      case "token_usage": {
        const call = latestCallForTurn(callsById, latestRequestByTurn, turnKey);
        if (!call) break;
        call.inputTokens = payload.input_tokens;
        call.cachedInputTokens = payload.cached_input_tokens ?? 0;
        call.cacheReadTokensReported =
          payload.cached_input_tokens !== undefined &&
          payload.cached_input_tokens !== null;
        call.cacheWriteTokens = payload.cache_write_tokens ?? 0;
        call.cacheWriteTokensReported =
          payload.cache_write_tokens !== undefined &&
          payload.cache_write_tokens !== null;
        call.outputTokens = payload.output_tokens;
        call.reasoningTokens = payload.reasoning_tokens ?? 0;
        call.totalTokens = payload.total_tokens;
        call.usageReceived = true;
        break;
      }
      case "error": {
        errorEventCount += 1;
        const call = latestCallForTurn(callsById, latestRequestByTurn, turnKey);
        if (call && !call.responseReceived && !call.usageReceived) {
          call.errored = true;
          call.completedAt = event.createdAt;
          call.durationMs = elapsedMs(call.startedAt, event.createdAt);
        }
        break;
      }
      case "tool_call_started": {
        toolCallCount += 1;
        const startedAt = timestampMs(event.createdAt);
        if (startedAt !== null) toolStarts.set(payload.call.id, startedAt);
        break;
      }
      case "tool_call_finished": {
        const completedAt = timestampMs(event.createdAt);
        const startedAt = toolStarts.get(payload.result.callId);
        if (startedAt !== undefined && completedAt !== null) {
          toolDurations.push(Math.max(0, completedAt - startedAt));
        }
        if (toolResultIsError(payload.result.metadata)) toolErrorCount += 1;
        break;
      }
    }
  }

  const calls = [...callsById.values()]
    .map((call): UsageCall => {
      const failed =
        call.errored ||
        (call.statusCode !== null && call.statusCode >= 400) ||
        (call.turnId !== null &&
          terminalTurns.has(call.turnId) &&
          !call.responseReceived &&
          !call.usageReceived);
      const succeeded = call.responseReceived || call.usageReceived;
      const {
        firstTokenAt: _,
        responseReceived: __,
        usageReceived: ___,
        errored: ____,
        ...result
      } = call;
      return {
        ...result,
        status: failed ? "failed" : succeeded ? "succeeded" : "running",
      };
    })
    .sort((left, right) => right.startedAt.localeCompare(left.startedAt));

  return {
    calls,
    summary: summarizeUsage(
      calls,
      includedTurns.size,
      errorEventCount,
      toolCallCount,
      toolErrorCount,
      toolDurations,
    ),
  };
}

function summarizeUsage(
  calls: UsageCall[],
  turnCount: number,
  errorEventCount: number,
  toolCallCount: number,
  toolErrorCount: number,
  toolDurations: number[],
): UsageSummary {
  const totals = calls.reduce(
    (current, call) => ({
      inputTokens: current.inputTokens + call.inputTokens,
      cachedInputTokens: current.cachedInputTokens + call.cachedInputTokens,
      cacheWriteTokens: current.cacheWriteTokens + call.cacheWriteTokens,
      outputTokens: current.outputTokens + call.outputTokens,
      reasoningTokens: current.reasoningTokens + call.reasoningTokens,
      totalTokens: current.totalTokens + call.totalTokens,
      retryCount: current.retryCount + call.retryCount,
    }),
    {
      inputTokens: 0,
      cachedInputTokens: 0,
      cacheWriteTokens: 0,
      outputTokens: 0,
      reasoningTokens: 0,
      totalTokens: 0,
      retryCount: 0,
    },
  );
  const completedDurations = calls.flatMap((call) =>
    call.durationMs === null ? [] : [call.durationMs],
  );
  const ttfts = calls.flatMap((call) =>
    call.ttftMs === null ? [] : [call.ttftMs],
  );
  const totalDurationMs = completedDurations.reduce(
    (total, duration) => total + duration,
    0,
  );
  const cacheReadCalls = calls.filter((call) => call.cacheReadTokensReported);
  const cacheReadInputTokens = cacheReadCalls.reduce(
    (total, call) => total + call.inputTokens,
    0,
  );
  const cacheWriteReportedRequestCount = calls.filter(
    (call) => call.cacheWriteTokensReported,
  ).length;

  return {
    requestCount: calls.length,
    successfulRequestCount: calls.filter((call) => call.status === "succeeded")
      .length,
    failedRequestCount: calls.filter((call) => call.status === "failed").length,
    runningRequestCount: calls.filter((call) => call.status === "running")
      .length,
    turnCount,
    ...totals,
    cacheReadInputTokens,
    cacheReadReportedRequestCount: cacheReadCalls.length,
    cacheWriteReportedRequestCount,
    cacheReadRatio:
      cacheReadCalls.length > 0
        ? ratio(totals.cachedInputTokens, cacheReadInputTokens)
        : null,
    averageTokensPerRequest: average(
      calls.map((call) => call.totalTokens).filter((tokens) => tokens > 0),
    ),
    averageLatencyMs: average(completedDurations),
    p95LatencyMs: percentile(completedDurations, 0.95),
    averageTtftMs: average(ttfts),
    outputTokensPerSecond:
      totalDurationMs > 0
        ? totals.outputTokens / (totalDurationMs / 1_000)
        : null,
    retryCount: totals.retryCount,
    retryRate: ratio(totals.retryCount, calls.length),
    errorEventCount,
    toolCallCount,
    toolErrorCount,
    averageToolDurationMs: average(toolDurations),
  };
}

function latestCallForTurn(
  callsById: Map<string, MutableUsageCall>,
  latestRequestByTurn: Map<string, string>,
  turnKey: string,
): MutableUsageCall | null {
  const requestId = latestRequestByTurn.get(turnKey);
  return requestId ? (callsById.get(requestId) ?? null) : null;
}

function timestampMs(value: string): number | null {
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : null;
}

function elapsedMs(start: string, end: string): number | null {
  const startMs = timestampMs(start);
  const endMs = timestampMs(end);
  return startMs === null || endMs === null
    ? null
    : Math.max(0, endMs - startMs);
}

function average(values: number[]): number | null {
  if (values.length === 0) return null;
  return values.reduce((total, value) => total + value, 0) / values.length;
}

function percentile(values: number[], fraction: number): number | null {
  if (values.length === 0) return null;
  const sorted = [...values].sort((left, right) => left - right);
  return sorted[Math.max(0, Math.ceil(sorted.length * fraction) - 1)] ?? null;
}

function ratio(numerator: number, denominator: number): number | null {
  return denominator > 0 ? numerator / denominator : null;
}

function toolResultIsError(metadata: unknown): boolean {
  if (!metadata || typeof metadata !== "object" || Array.isArray(metadata)) {
    return false;
  }
  const value = metadata as Record<string, unknown>;
  return value.success === false || value.isError === true;
}
