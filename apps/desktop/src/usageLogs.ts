import type {
  AgentEvent,
  ModelCallPurpose,
  ModelContextItem,
  ModelRequestSnapshot,
  ThreadModelSelection,
  TokenEstimateBreakdown,
} from "./types";
import { toolExecutionDurationMs } from "./toolExecutionTiming.ts";

export type UsageCallStatus = "running" | "succeeded" | "failed";

export type CacheReuseState = "reused" | "degraded" | "broken" | "unverified";

export type CacheBreakReason =
  | "content_changed"
  | "tool_catalog_changed"
  | "system_prompt_changed"
  | "cache_key_changed"
  | "model_changed"
  | "provider_changed"
  | "input_below_minimum"
  | "stateful_context"
  | "operational_miss"
  | "cache_hit"
  | "no_baseline"
  | "usage_unreported";

export type CacheBreakPoint = {
  kind:
    | ModelContextItem["kind"]
    | "tool_catalog"
    | "system_prompt"
    | "cache_key"
    | "model"
    | "provider"
    | "input";
  source: string;
  cacheScope: ModelContextItem["cacheScope"] | null;
  change: "changed" | "inserted" | "removed" | "configuration";
  tokenOffsetEstimate: number | null;
  previousTokenEstimate: number | null;
  currentTokenEstimate: number | null;
};

export type CacheReuseDiagnostic = {
  state: CacheReuseState;
  reason: CacheBreakReason;
  confidence: "high" | "medium" | "low";
  previousRequestId: string | null;
  previousCachedInputTokens: number | null;
  currentCachedInputTokens: number;
  lostCachedTokens: number;
  breakpoint: CacheBreakPoint | null;
};

export type UsageCall = {
  id: string;
  turnId: string | null;
  round: number;
  purpose: ModelCallPurpose;
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
  retryReasons: string[];
  contextTokenEstimate: number | null;
  inputBreakdown: TokenEstimateBreakdown | null;
  rawEstimateErrorRatio: number | null;
  estimateErrorRatio: number | null;
  estimatedRetryInputTokens: number;
  providerUsageReported: boolean;
  inputTokens: number;
  cachedInputTokens: number;
  cacheReadTokensReported: boolean;
  cacheWriteTokens: number;
  cacheWriteTokensReported: boolean;
  outputTokens: number;
  reasoningTokens: number;
  totalTokens: number;
  cacheReuse: CacheReuseDiagnostic;
};

export type UsageSummary = {
  requestCount: number;
  successfulRequestCount: number;
  failedRequestCount: number;
  runningRequestCount: number;
  turnCount: number;
  successfulTurnCount: number;
  failedTurnCount: number;
  inputTokens: number;
  cachedInputTokens: number;
  cacheReadInputTokens: number;
  cacheReadReportedRequestCount: number;
  cacheWriteTokens: number;
  cacheWriteReportedRequestCount: number;
  cacheBreakCount: number;
  cacheDegradationCount: number;
  outputTokens: number;
  reasoningTokens: number;
  totalTokens: number;
  uncachedInputTokens: number;
  localInputEstimate: number;
  rawLocalInputEstimate: number;
  estimateCalibrationFactor: number | null;
  tokenBreakdown: TokenEstimateBreakdown;
  providerUsageReportedRequestCount: number;
  providerUsageCoverage: number | null;
  estimateErrorMean: number | null;
  estimateErrorP95: number | null;
  rawEstimateErrorMean: number | null;
  rawEstimateErrorP95: number | null;
  tokensPerSuccessfulTask: number | null;
  uncachedTokensPerSuccessfulTask: number | null;
  estimatedRetryInputTokens: number;
  compatibilityRetryCount: number;
  invalidToolLoopCount: number;
  finalizationGuardRejectCount: number;
  noProgressSignalCount: number;
  duplicatePlanCount: number;
  compactionRequestCount: number;
  compactionTokens: number;
  cacheReadRatio: number | null;
  averageTokensPerRequest: number | null;
  averageLatencyMs: number | null;
  p95LatencyMs: number | null;
  averageTtftMs: number | null;
  outputTokensPerSecond: number | null;
  totalModelDurationMs: number;
  retryCount: number;
  retryRate: number | null;
  errorEventCount: number;
  toolCallCount: number;
  toolErrorCount: number;
  averageToolDurationMs: number | null;
  totalToolDurationMs: number;
};

export type UsageDashboardData = {
  calls: UsageCall[];
  cacheBreaks: UsageCall[];
  summary: UsageSummary;
};

type CacheRequestObservation = {
  contextHash: string | null;
  contextItems: ModelContextItem[];
  promptCacheKey: string | null;
  previousResponseId: string | null;
  systemPrompt: string | null;
  toolCatalogSignature: string | null;
};

type UsageWasteSignals = {
  compatibilityRetryCount: number;
  invalidToolLoopCount: number;
  finalizationGuardRejectCount: number;
  noProgressSignalCount: number;
  duplicatePlanCount: number;
};

type MutableUsageCall = UsageCall & {
  firstTokenAt: string | null;
  responseReceived: boolean;
  usageReceived: boolean;
  errored: boolean;
  cacheObservation: CacheRequestObservation;
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
  const inputBreakdownByRequest = new Map<string, TokenEstimateBreakdown>();
  const purposeByRequest = new Map<string, ModelCallPurpose>();
  const cacheObservationByRequest = new Map<string, CacheRequestObservation>();
  const callsById = new Map<string, MutableUsageCall>();
  const latestRequestByTurn = new Map<string, string>();
  const includedTurns = new Set<string>();
  const terminalTurns = new Set<string>();
  const successfulTurns = new Set<string>();
  const failedTurns = new Set<string>();
  const latestPlanSignatureByTurn = new Map<string, string>();
  const toolStarts = new Map<string, number>();
  const toolDurations: number[] = [];
  let errorEventCount = 0;
  let toolCallCount = 0;
  let toolErrorCount = 0;
  const wasteSignals: UsageWasteSignals = {
    compatibilityRetryCount: 0,
    invalidToolLoopCount: 0,
    finalizationGuardRejectCount: 0,
    noProgressSignalCount: 0,
    duplicatePlanCount: 0,
  };

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
      if (payload.token_breakdown) {
        inputBreakdownByRequest.set(
          payload.request_id,
          payload.token_breakdown,
        );
      }
      purposeByRequest.set(
        payload.request_id,
        payload.purpose ?? "agent_round",
      );
      const observation = getCacheObservation(
        cacheObservationByRequest,
        payload.request_id,
      );
      observation.contextHash = payload.context_hash;
      observation.contextItems = payload.items ?? [];
    }

    if (payload.type === "model_request" && payload.request) {
      const observation = getCacheObservation(
        cacheObservationByRequest,
        payload.request_id,
      );
      captureModelRequestObservation(observation, payload.request);
    }

    const turnKey = event.turnId ?? "__thread";

    if (payload.type === "turn_started" && event.turnId) {
      includedTurns.add(event.turnId);
    }

    if (terminalTurnEvents.has(payload.type) && event.turnId) {
      terminalTurns.add(event.turnId);
      includedTurns.add(event.turnId);
    }

    if (payload.type === "turn_finished" && event.turnId) {
      successfulTurns.add(event.turnId);
      failedTurns.delete(event.turnId);
    } else if (
      (payload.type === "turn_cancelled" || payload.type === "error") &&
      event.turnId
    ) {
      failedTurns.add(event.turnId);
      successfulTurns.delete(event.turnId);
    }

    if (payload.type === "context_warning") {
      if (payload.stage === "invalid_tool_call_circuit_breaker") {
        wasteSignals.invalidToolLoopCount += 1;
      } else if (payload.stage === "finalization_guard") {
        wasteSignals.finalizationGuardRejectCount += 1;
      } else if (payload.stage === "step_reminder.repeated_tool_calls") {
        wasteSignals.noProgressSignalCount += 1;
      }
    }

    if (payload.type === "work_form_updated") {
      const signature = stableSerialize(payload.form);
      if (latestPlanSignatureByTurn.get(turnKey) === signature) {
        wasteSignals.duplicatePlanCount += 1;
      }
      latestPlanSignatureByTurn.set(turnKey, signature);
    }

    switch (payload.type) {
      case "provider_request_sent": {
        if (event.turnId) includedTurns.add(event.turnId);
        const call: MutableUsageCall = {
          id: payload.request_id,
          turnId: event.turnId ?? null,
          round: payload.round,
          purpose: purposeByRequest.get(payload.request_id) ?? "agent_round",
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
          retryReasons: [],
          contextTokenEstimate:
            contextEstimateByRequest.get(payload.request_id) ?? null,
          inputBreakdown:
            inputBreakdownByRequest.get(payload.request_id) ?? null,
          rawEstimateErrorRatio: null,
          estimateErrorRatio: null,
          estimatedRetryInputTokens: 0,
          providerUsageReported: false,
          inputTokens: 0,
          cachedInputTokens: 0,
          cacheReadTokensReported: false,
          cacheWriteTokens: 0,
          cacheWriteTokensReported: false,
          outputTokens: 0,
          reasoningTokens: 0,
          totalTokens: 0,
          cacheReuse: emptyCacheDiagnostic(),
          firstTokenAt: null,
          responseReceived: false,
          usageReceived: false,
          errored: false,
          cacheObservation:
            cacheObservationByRequest.get(payload.request_id) ??
            emptyCacheObservation(),
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
        call.retryReasons.push(payload.reason);
        call.estimatedRetryInputTokens =
          (call.contextTokenEstimate ?? 0) * call.retryCount;
        if (isCompatibilityRetry(payload.reason)) {
          wasteSignals.compatibilityRetryCount += 1;
        }
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
      case "model_delta":
      case "reasoning_delta": {
        const call = latestCallForTurn(callsById, latestRequestByTurn, turnKey);
        if (!call || call.firstTokenAt) break;
        call.firstTokenAt = event.createdAt;
        call.ttftMs = elapsedMs(call.startedAt, event.createdAt);
        break;
      }
      case "token_usage": {
        const call = payload.request_id
          ? callsById.get(payload.request_id)
          : latestCallForTurn(callsById, latestRequestByTurn, turnKey);
        if (!call) break;
        call.purpose = payload.purpose ?? call.purpose;
        call.contextTokenEstimate =
          payload.local_input_estimate ?? call.contextTokenEstimate;
        call.inputBreakdown = payload.input_breakdown ?? call.inputBreakdown;
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
        call.estimateErrorRatio = relativeEstimateError(
          call.contextTokenEstimate,
          call.inputTokens,
        );
        call.rawEstimateErrorRatio = relativeEstimateError(
          call.inputBreakdown?.total ?? null,
          call.inputTokens,
        );
        call.estimatedRetryInputTokens =
          (call.contextTokenEstimate ?? 0) * call.retryCount;
        call.providerUsageReported = true;
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
        const recordedDurationMs = toolExecutionDurationMs(payload.result);
        if (recordedDurationMs !== null) {
          toolDurations.push(recordedDurationMs);
        } else if (startedAt !== undefined && completedAt !== null) {
          toolDurations.push(Math.max(0, completedAt - startedAt));
        }
        if (toolResultIsError(payload.result.metadata)) toolErrorCount += 1;
        break;
      }
    }
  }

  const chronologicalCalls = [...callsById.values()].sort((left, right) =>
    left.startedAt.localeCompare(right.startedAt),
  );
  let previousReportedCall: MutableUsageCall | null = null;
  const calls = chronologicalCalls
    .map((call): UsageCall => {
      const failed =
        call.errored ||
        (call.statusCode !== null && call.statusCode >= 400) ||
        (call.turnId !== null &&
          terminalTurns.has(call.turnId) &&
          !call.responseReceived &&
          !call.usageReceived);
      const succeeded = call.responseReceived || call.usageReceived;
      const cacheReuse =
        call.purpose === "agent_round"
          ? diagnoseCacheReuse(call, previousReportedCall)
          : emptyCacheDiagnostic();
      if (
        call.purpose === "agent_round" &&
        call.cacheReadTokensReported &&
        call.usageReceived
      ) {
        previousReportedCall = call;
      }
      const {
        firstTokenAt: _,
        responseReceived: __,
        usageReceived: ___,
        errored: ____,
        cacheObservation: _____,
        ...result
      } = call;
      return {
        ...result,
        cacheReuse,
        status: failed ? "failed" : succeeded ? "succeeded" : "running",
      };
    })
    .sort((left, right) => right.startedAt.localeCompare(left.startedAt));

  return {
    calls,
    cacheBreaks: calls.filter(
      (call) =>
        call.cacheReuse.state === "broken" ||
        call.cacheReuse.state === "degraded",
    ),
    summary: summarizeUsage(
      calls,
      includedTurns.size,
      successfulTurns.size,
      failedTurns.size,
      errorEventCount,
      toolCallCount,
      toolErrorCount,
      toolDurations,
      wasteSignals,
    ),
  };
}

function summarizeUsage(
  calls: UsageCall[],
  turnCount: number,
  successfulTurnCount: number,
  failedTurnCount: number,
  errorEventCount: number,
  toolCallCount: number,
  toolErrorCount: number,
  toolDurations: number[],
  wasteSignals: UsageWasteSignals,
): UsageSummary {
  const tokenBreakdown = emptyTokenBreakdown();
  const totals = calls.reduce(
    (current, call) => {
      if (call.inputBreakdown) {
        addTokenBreakdown(tokenBreakdown, call.inputBreakdown);
      }
      return {
        inputTokens: current.inputTokens + call.inputTokens,
        cachedInputTokens: current.cachedInputTokens + call.cachedInputTokens,
        cacheWriteTokens: current.cacheWriteTokens + call.cacheWriteTokens,
        outputTokens: current.outputTokens + call.outputTokens,
        reasoningTokens: current.reasoningTokens + call.reasoningTokens,
        totalTokens: current.totalTokens + call.totalTokens,
        retryCount: current.retryCount + call.retryCount,
        localInputEstimate:
          current.localInputEstimate + (call.contextTokenEstimate ?? 0),
        rawLocalInputEstimate:
          current.rawLocalInputEstimate + (call.inputBreakdown?.total ?? 0),
        estimatedRetryInputTokens:
          current.estimatedRetryInputTokens + call.estimatedRetryInputTokens,
      };
    },
    {
      inputTokens: 0,
      cachedInputTokens: 0,
      cacheWriteTokens: 0,
      outputTokens: 0,
      reasoningTokens: 0,
      totalTokens: 0,
      retryCount: 0,
      localInputEstimate: 0,
      rawLocalInputEstimate: 0,
      estimatedRetryInputTokens: 0,
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
  const successfulRequestCount = calls.filter(
    (call) => call.status === "succeeded",
  ).length;
  const providerUsageReportedRequestCount = calls.filter(
    (call) => call.status === "succeeded" && call.providerUsageReported,
  ).length;
  const estimateErrors = calls.flatMap((call) =>
    call.estimateErrorRatio === null ? [] : [call.estimateErrorRatio],
  );
  const rawEstimateErrors = calls.flatMap((call) =>
    call.rawEstimateErrorRatio === null ? [] : [call.rawEstimateErrorRatio],
  );
  const uncachedInputTokens = calls.reduce(
    (total, call) =>
      total + Math.max(0, call.inputTokens - call.cachedInputTokens),
    0,
  );
  const compactionCalls = calls.filter(
    (call) => call.purpose === "context_compaction",
  );

  return {
    requestCount: calls.length,
    successfulRequestCount,
    failedRequestCount: calls.filter((call) => call.status === "failed").length,
    runningRequestCount: calls.filter((call) => call.status === "running")
      .length,
    turnCount,
    successfulTurnCount,
    failedTurnCount,
    ...totals,
    uncachedInputTokens,
    tokenBreakdown,
    estimateCalibrationFactor: ratio(
      totals.localInputEstimate,
      totals.rawLocalInputEstimate,
    ),
    providerUsageReportedRequestCount,
    providerUsageCoverage: ratio(
      providerUsageReportedRequestCount,
      successfulRequestCount,
    ),
    estimateErrorMean: average(estimateErrors),
    estimateErrorP95: percentile(estimateErrors, 0.95),
    rawEstimateErrorMean: average(rawEstimateErrors),
    rawEstimateErrorP95: percentile(rawEstimateErrors, 0.95),
    tokensPerSuccessfulTask: ratio(totals.totalTokens, successfulTurnCount),
    uncachedTokensPerSuccessfulTask: ratio(
      uncachedInputTokens + totals.outputTokens,
      successfulTurnCount,
    ),
    ...wasteSignals,
    compactionRequestCount: compactionCalls.length,
    compactionTokens: compactionCalls.reduce(
      (total, call) => total + call.totalTokens,
      0,
    ),
    cacheReadInputTokens,
    cacheReadReportedRequestCount: cacheReadCalls.length,
    cacheWriteReportedRequestCount,
    cacheBreakCount: calls.filter((call) => call.cacheReuse.state === "broken")
      .length,
    cacheDegradationCount: calls.filter(
      (call) => call.cacheReuse.state === "degraded",
    ).length,
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
    totalModelDurationMs: totalDurationMs,
    retryCount: totals.retryCount,
    retryRate: ratio(totals.retryCount, calls.length),
    errorEventCount,
    toolCallCount,
    toolErrorCount,
    averageToolDurationMs: average(toolDurations),
    totalToolDurationMs: toolDurations.reduce(
      (total, duration) => total + duration,
      0,
    ),
  };
}

function getCacheObservation(
  observations: Map<string, CacheRequestObservation>,
  requestId: string,
): CacheRequestObservation {
  const existing = observations.get(requestId);
  if (existing) return existing;
  const observation = emptyCacheObservation();
  observations.set(requestId, observation);
  return observation;
}

function emptyCacheObservation(): CacheRequestObservation {
  return {
    contextHash: null,
    contextItems: [],
    promptCacheKey: null,
    previousResponseId: null,
    systemPrompt: null,
    toolCatalogSignature: null,
  };
}

function captureModelRequestObservation(
  observation: CacheRequestObservation,
  request: ModelRequestSnapshot,
): void {
  observation.promptCacheKey = request.promptCacheKey ?? null;
  observation.previousResponseId = request.previousResponseId ?? null;
  observation.systemPrompt = request.systemPrompt ?? null;
  observation.toolCatalogSignature = stableSerialize(
    request.toolCandidates ?? [],
  );
}

function emptyCacheDiagnostic(): CacheReuseDiagnostic {
  return {
    state: "unverified",
    reason: "usage_unreported",
    confidence: "low",
    previousRequestId: null,
    previousCachedInputTokens: null,
    currentCachedInputTokens: 0,
    lostCachedTokens: 0,
    breakpoint: null,
  };
}

function diagnoseCacheReuse(
  current: MutableUsageCall,
  previous: MutableUsageCall | null,
): CacheReuseDiagnostic {
  const base = {
    previousRequestId: previous?.id ?? null,
    previousCachedInputTokens: previous?.cachedInputTokens ?? null,
    currentCachedInputTokens: current.cachedInputTokens,
    lostCachedTokens: 0,
    breakpoint: null,
  } satisfies Pick<
    CacheReuseDiagnostic,
    | "previousRequestId"
    | "previousCachedInputTokens"
    | "currentCachedInputTokens"
    | "lostCachedTokens"
    | "breakpoint"
  >;

  if (!current.cacheReadTokensReported) {
    return {
      ...base,
      state: "unverified",
      reason: "usage_unreported",
      confidence: "low",
    };
  }
  if (!previous || previous.cachedInputTokens === 0) {
    return {
      ...base,
      state: current.cachedInputTokens > 0 ? "reused" : "unverified",
      reason: current.cachedInputTokens > 0 ? "cache_hit" : "no_baseline",
      confidence: current.cachedInputTokens > 0 ? "high" : "low",
    };
  }

  const expectedReusableTokens = Math.min(
    previous.cachedInputTokens,
    current.inputTokens,
  );
  const lostCachedTokens = Math.max(
    0,
    expectedReusableTokens - current.cachedInputTokens,
  );
  if (lostCachedTokens === 0) {
    return {
      ...base,
      state: "reused",
      reason: "cache_hit",
      confidence: "high",
    };
  }

  const state = current.cachedInputTokens === 0 ? "broken" : "degraded";
  const shared = { ...base, state, lostCachedTokens } as const;
  if (isOpenAiCall(current) && current.inputTokens < 1_024) {
    return {
      ...shared,
      reason: "input_below_minimum",
      confidence: "high",
      breakpoint: configurationBreakpoint("input", "input_tokens"),
    };
  }
  if (changedKnownValue(previous.providerId, current.providerId)) {
    return {
      ...shared,
      reason: "provider_changed",
      confidence: "high",
      breakpoint: configurationBreakpoint(
        "provider",
        current.providerId ?? "provider",
      ),
    };
  }
  if (changedKnownValue(previous.model, current.model)) {
    return {
      ...shared,
      reason: "model_changed",
      confidence: "high",
      breakpoint: configurationBreakpoint("model", current.model ?? "model"),
    };
  }
  if (
    previous.cacheObservation.promptCacheKey !==
      current.cacheObservation.promptCacheKey &&
    (previous.cacheObservation.promptCacheKey !== null ||
      current.cacheObservation.promptCacheKey !== null)
  ) {
    return {
      ...shared,
      reason: "cache_key_changed",
      confidence: "high",
      breakpoint: configurationBreakpoint(
        "cache_key",
        current.cacheObservation.promptCacheKey ?? "未设置 prompt_cache_key",
      ),
    };
  }
  if (
    changedKnownValue(
      previous.cacheObservation.toolCatalogSignature,
      current.cacheObservation.toolCatalogSignature,
    )
  ) {
    return {
      ...shared,
      reason: "tool_catalog_changed",
      confidence: "high",
      breakpoint: configurationBreakpoint("tool_catalog", "tool_candidates"),
    };
  }
  const breakpoint = findContextBreakpoint(
    previous.cacheObservation.contextItems,
    current.cacheObservation.contextItems,
  );
  if (breakpoint) {
    return {
      ...shared,
      reason: "content_changed",
      confidence: "medium",
      breakpoint,
    };
  }
  if (
    changedKnownValue(
      previous.cacheObservation.systemPrompt,
      current.cacheObservation.systemPrompt,
    )
  ) {
    return {
      ...shared,
      reason: "system_prompt_changed",
      confidence: "high",
      breakpoint: configurationBreakpoint("system_prompt", "system_prompt"),
    };
  }
  if (current.cacheObservation.previousResponseId) {
    return {
      ...shared,
      reason: "stateful_context",
      confidence: "low",
    };
  }
  return {
    ...shared,
    reason: "operational_miss",
    confidence: "low",
  };
}

function findContextBreakpoint(
  previous: ModelContextItem[],
  current: ModelContextItem[],
): CacheBreakPoint | null {
  if (previous.length === 0 || current.length === 0) return null;
  let index = 0;
  while (
    index < previous.length &&
    index < current.length &&
    contextItemMatches(previous[index], current[index])
  ) {
    index += 1;
  }
  if (index === previous.length) {
    return null;
  }

  const previousItem = previous[index];
  const currentItem = current[index];
  const tokenOffsetEstimate = current
    .slice(0, index)
    .reduce((total, item) => total + item.tokenEstimate, 0);
  if (!currentItem) {
    return contextItemBreakpoint(
      previousItem,
      "removed",
      tokenOffsetEstimate,
      previousItem?.tokenEstimate ?? null,
      null,
    );
  }
  if (
    current[index + 1] &&
    previousItem &&
    contextItemMatches(previousItem, current[index + 1])
  ) {
    return contextItemBreakpoint(
      currentItem,
      "inserted",
      tokenOffsetEstimate,
      null,
      currentItem.tokenEstimate,
    );
  }
  if (
    previous[index + 1] &&
    contextItemMatches(previous[index + 1], currentItem)
  ) {
    return contextItemBreakpoint(
      previousItem,
      "removed",
      tokenOffsetEstimate,
      previousItem?.tokenEstimate ?? null,
      null,
    );
  }
  return contextItemBreakpoint(
    currentItem,
    "changed",
    tokenOffsetEstimate,
    previousItem?.tokenEstimate ?? null,
    currentItem.tokenEstimate,
  );
}

function contextItemMatches(
  left: ModelContextItem | undefined,
  right: ModelContextItem | undefined,
): boolean {
  return Boolean(
    left &&
    right &&
    left.kind === right.kind &&
    left.role === right.role &&
    left.authority === right.authority &&
    left.source === right.source &&
    left.cacheScope === right.cacheScope &&
    left.contentHash === right.contentHash,
  );
}

function contextItemBreakpoint(
  item: ModelContextItem | undefined,
  change: CacheBreakPoint["change"],
  tokenOffsetEstimate: number,
  previousTokenEstimate: number | null,
  currentTokenEstimate: number | null,
): CacheBreakPoint | null {
  if (!item) return null;
  return {
    kind: item.kind,
    source: item.source,
    cacheScope: item.cacheScope,
    change,
    tokenOffsetEstimate,
    previousTokenEstimate,
    currentTokenEstimate,
  };
}

function configurationBreakpoint(
  kind: Extract<
    CacheBreakPoint["kind"],
    | "tool_catalog"
    | "system_prompt"
    | "cache_key"
    | "model"
    | "provider"
    | "input"
  >,
  source: string,
): CacheBreakPoint {
  return {
    kind,
    source,
    cacheScope: null,
    change: "configuration",
    tokenOffsetEstimate: null,
    previousTokenEstimate: null,
    currentTokenEstimate: null,
  };
}

function changedKnownValue(
  previous: string | null,
  current: string | null,
): boolean {
  return previous !== null && current !== null && previous !== current;
}

function isOpenAiCall(call: UsageCall): boolean {
  return call.adapter.toLowerCase().includes("openai");
}

function isCompatibilityRetry(reason: string): boolean {
  return /(compatib|previous[_ ]response|stored response|cursor|replay|fallback)/i.test(
    reason,
  );
}

function relativeEstimateError(
  estimate: number | null,
  actual: number,
): number | null {
  if (estimate === null || actual <= 0) return null;
  return Math.abs(estimate - actual) / actual;
}

function emptyTokenBreakdown(): TokenEstimateBreakdown {
  return {
    baseInstructions: 0,
    developerInstructions: 0,
    repositoryInstructions: 0,
    runtimeContext: 0,
    skillInstructions: 0,
    summaries: 0,
    checkpoints: 0,
    conversation: 0,
    currentUser: 0,
    toolCalls: 0,
    toolResults: 0,
    directToolSchemas: 0,
    deferredToolCatalog: 0,
    loadedToolSchemas: 0,
    toolSchemas: 0,
    providerState: 0,
    other: 0,
    total: 0,
  };
}

function addTokenBreakdown(
  target: TokenEstimateBreakdown,
  source: TokenEstimateBreakdown,
): void {
  target.baseInstructions += source.baseInstructions;
  target.developerInstructions += source.developerInstructions;
  target.repositoryInstructions += source.repositoryInstructions;
  target.runtimeContext += source.runtimeContext;
  target.skillInstructions += source.skillInstructions;
  target.summaries += source.summaries;
  target.checkpoints += source.checkpoints;
  target.conversation += source.conversation;
  target.currentUser += source.currentUser;
  target.toolCalls += source.toolCalls;
  target.toolResults += source.toolResults;
  target.directToolSchemas =
    (target.directToolSchemas ?? 0) + (source.directToolSchemas ?? 0);
  target.deferredToolCatalog =
    (target.deferredToolCatalog ?? 0) + (source.deferredToolCatalog ?? 0);
  target.loadedToolSchemas =
    (target.loadedToolSchemas ?? 0) + (source.loadedToolSchemas ?? 0);
  target.toolSchemas += source.toolSchemas;
  target.providerState += source.providerState;
  target.other += source.other;
  target.total += source.total;
}

function stableSerialize(value: unknown): string {
  if (value === null || typeof value !== "object") {
    return JSON.stringify(value) ?? String(value);
  }
  if (Array.isArray(value)) {
    return `[${value.map(stableSerialize).join(",")}]`;
  }
  const record = value as Record<string, unknown>;
  return `{${Object.keys(record)
    .sort()
    .map((key) => `${JSON.stringify(key)}:${stableSerialize(record[key])}`)
    .join(",")}}`;
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
