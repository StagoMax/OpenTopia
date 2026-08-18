import {
  formatMetricDuration,
  formatMetricPercent,
  formatMetricTokenCount,
  formatMetricTokenRate,
  type ConversationMetrics,
} from "../../conversationMetrics";

export function ComposerMetrics({ metrics }: { metrics: ConversationMetrics }) {
  const groups = [
    `${metrics.turnCount.toLocaleString()} 轮 · ${metrics.stepCount.toLocaleString()} 步`,
    `LLM ${formatMetricDuration(metrics.modelDurationMs)} · 工具调用 ${formatMetricDuration(metrics.toolDurationMs)}`,
    `首 token 平均 ${formatMetricDuration(metrics.averageTtftMs)} · ${formatMetricTokenRate(metrics.outputTokensPerSecond)}`,
    `缓存命中 ${formatMetricPercent(metrics.cacheReadRatio)}`,
    `输入 ${formatMetricTokenCount(metrics.inputTokens)} · 输出 ${formatMetricTokenCount(metrics.outputTokens)}`,
  ];
  const exactTokenCounts = `输入 ${metrics.inputTokens.toLocaleString()} tok · 输出 ${metrics.outputTokens.toLocaleString()} tok`;

  return (
    <div
      className="composer-metrics"
      aria-label={`对话运行指标：${groups.slice(0, -1).join("；")}；${exactTokenCounts}`}
      title={`${groups.slice(0, -1).join(" | ")} | ${exactTokenCounts}`}
    >
      {groups.map((group) => (
        <span className="composer-metric-group" key={group}>
          {group}
        </span>
      ))}
    </div>
  );
}
