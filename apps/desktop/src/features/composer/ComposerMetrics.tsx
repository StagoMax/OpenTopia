import {
  formatMetricDuration,
  formatMetricPercent,
  formatMetricTokenCount,
  formatMetricTokenRate,
  type ConversationMetrics,
} from "../../conversationMetrics";

export function ComposerMetrics({
  metrics,
  showContextWindowUsage = false,
}: {
  metrics: ConversationMetrics;
  showContextWindowUsage?: boolean;
}) {
  const contextUsage = metrics.contextWindowUsage;
  const contextUsageLabel = contextUsage
    ? formatMetricPercent(contextUsage.ratio)
    : "—";
  const tokenGroup = `输入 ${formatMetricTokenCount(metrics.inputTokens)} · 输出 ${formatMetricTokenCount(metrics.outputTokens)}`;
  const groups = [
    `${metrics.turnCount.toLocaleString()} 轮 · ${metrics.stepCount.toLocaleString()} 步`,
    `LLM ${formatMetricDuration(metrics.modelDurationMs)} · 工具调用 ${formatMetricDuration(metrics.toolDurationMs)}`,
    `TTFT ${formatMetricDuration(metrics.averageTtftMs)} · ${formatMetricTokenRate(metrics.outputTokensPerSecond)}`,
    `缓存命中 ${formatMetricPercent(metrics.cacheReadRatio)}`,
  ];
  const exactTokenCounts = `输入 ${metrics.inputTokens.toLocaleString()} tok · 输出 ${metrics.outputTokens.toLocaleString()} tok`;
  const contextUsageDescription = contextUsage
    ? `上下文窗口 ${contextUsage.usedTokens.toLocaleString()} / ${contextUsage.totalTokens.toLocaleString()} tok (${contextUsageLabel})`
    : "上下文窗口用量不可用";
  const exactContextUsage = showContextWindowUsage
    ? ` · ${contextUsageDescription}`
    : "";
  const exactGroups = [...groups, `${exactTokenCounts}${exactContextUsage}`];

  return (
    <div
      className="composer-metrics"
      aria-label={`对话运行指标：${exactGroups.join("；")}`}
      title={exactGroups.join(" | ")}
    >
      {groups.map((group) => (
        <span className="composer-metric-group" key={group}>
          {group}
        </span>
      ))}
      <span className="composer-metric-group">{tokenGroup}</span>
      {showContextWindowUsage ? (
        <ContextWindowUsageRing
          usage={contextUsage}
          label={contextUsageDescription}
        />
      ) : null}
    </div>
  );
}

function ContextWindowUsageRing({
  usage,
  label,
}: {
  usage: ConversationMetrics["contextWindowUsage"];
  label: string;
}) {
  const progress = usage ? Math.min(100, Math.max(0, usage.ratio * 100)) : 0;

  return (
    <span
      className="composer-metric-group composer-context-usage"
      role="img"
      aria-label={label}
      title={label}
    >
      <svg
        className="composer-context-usage-ring"
        viewBox="0 0 14 14"
        aria-hidden="true"
      >
        <circle
          className="composer-context-usage-track"
          cx="7"
          cy="7"
          r="5"
          pathLength="100"
        />
        {usage ? (
          <circle
            className="composer-context-usage-value"
            cx="7"
            cy="7"
            r="5"
            pathLength="100"
            strokeDasharray={`${progress} 100`}
          />
        ) : null}
      </svg>
    </span>
  );
}
