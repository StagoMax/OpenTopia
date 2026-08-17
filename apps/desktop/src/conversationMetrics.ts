import type { AgentEvent, ThreadModelSelection } from "./types";
import { aggregateUsageEvents } from "./usageLogs.ts";

export type ConversationMetrics = {
  turnCount: number;
  stepCount: number;
  modelDurationMs: number;
  toolDurationMs: number;
  averageTtftMs: number | null;
  outputTokensPerSecond: number | null;
  cacheReadRatio: number | null;
  inputTokens: number;
  outputTokens: number;
};

export function conversationMetrics(
  events: AgentEvent[],
  fallbackModelSelection?: ThreadModelSelection | null,
): ConversationMetrics | null {
  const { summary } = aggregateUsageEvents(events, {
    fallbackModelSelection,
  });
  if (
    summary.turnCount === 0 &&
    summary.requestCount === 0 &&
    summary.toolCallCount === 0
  ) {
    return null;
  }

  return {
    turnCount: summary.turnCount,
    stepCount: summary.requestCount,
    modelDurationMs: summary.totalModelDurationMs,
    toolDurationMs: summary.totalToolDurationMs,
    averageTtftMs: summary.averageTtftMs,
    outputTokensPerSecond: summary.outputTokensPerSecond,
    cacheReadRatio: summary.cacheReadRatio,
    inputTokens: summary.inputTokens,
    outputTokens: summary.outputTokens,
  };
}

export function formatMetricDuration(durationMs: number | null): string {
  if (durationMs === null || !Number.isFinite(durationMs)) return "—";
  const safeDurationMs = Math.max(0, durationMs);
  if (safeDurationMs < 1_000) {
    return `${Math.round(safeDurationMs).toLocaleString("en-US")}ms`;
  }

  const totalSeconds = Math.round(safeDurationMs / 1_000);
  if (totalSeconds < 60) {
    const seconds = Math.round(safeDurationMs / 100) / 10;
    return `${seconds.toLocaleString("en-US", {
      maximumFractionDigits: 1,
    })}s`;
  }

  const hours = Math.floor(totalSeconds / 3_600);
  const minutes = Math.floor((totalSeconds % 3_600) / 60);
  const seconds = totalSeconds % 60;
  return [
    hours > 0 ? `${hours}h` : null,
    minutes > 0 || hours > 0 ? `${minutes}m` : null,
    `${seconds}s`,
  ]
    .filter(Boolean)
    .join(" ");
}

export function formatMetricTokenCount(tokens: number): string {
  const safeTokens = Number.isFinite(tokens) ? Math.max(0, tokens) : 0;
  return `${new Intl.NumberFormat("en-US", {
    notation: "compact",
    maximumFractionDigits: 1,
  }).format(safeTokens)} tok`;
}

export function formatMetricTokenRate(tokensPerSecond: number | null): string {
  if (tokensPerSecond === null || !Number.isFinite(tokensPerSecond)) return "—";
  return `${tokensPerSecond.toLocaleString("en-US", {
    maximumFractionDigits: tokensPerSecond >= 100 ? 0 : 1,
  })} tok/s`;
}

export function formatMetricPercent(ratio: number | null): string {
  if (ratio === null || !Number.isFinite(ratio)) return "—";
  return `${(Math.max(0, ratio) * 100).toLocaleString("en-US", {
    maximumFractionDigits: 1,
  })}%`;
}
