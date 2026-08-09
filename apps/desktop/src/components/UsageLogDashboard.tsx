import { useMemo } from "react";
import { Activity, CircleCheck, TriangleAlert } from "lucide-react";

import {
  aggregateUsageEvents,
  type CacheReuseDiagnostic,
  type UsageCall,
  type UsageSummary,
} from "../usageLogs";
import type { AgentEvent, Thread } from "../types";
import { Badge, Panel, type BadgeVariant } from "./ui";
import "./UsageLogDashboard.css";

type UsageLogDashboardProps = {
  thread: Thread;
  events: AgentEvent[];
  isLoading: boolean;
};

export function UsageLogDashboard({
  thread,
  events,
  isLoading,
}: UsageLogDashboardProps) {
  const data = useMemo(
    () =>
      aggregateUsageEvents(events, {
        fallbackModelSelection: thread.modelSelection,
      }),
    [events, thread.modelSelection],
  );
  return (
    <div className="usage-log-dashboard">
      <header className="usage-log-header">
        <div className="usage-log-heading">
          <div>
            <div className="usage-log-title-row">
              <Activity size={16} aria-hidden="true" />
              <h2>使用日志</h2>
              <Badge variant="info">API 可观测</Badge>
            </div>
            <p>{thread.title}</p>
          </div>
        </div>
      </header>

      <div className="usage-log-scroll">
        <section className="usage-kpi-grid" aria-label="Token 使用概览">
          <MetricCard
            label="总 Tokens"
            value={formatInteger(data.summary.totalTokens)}
            detail={`${formatInteger(data.summary.tokensPerSuccessfulTask)} / 成功任务`}
          />
          <MetricCard
            label="输入 Tokens"
            value={formatInteger(data.summary.inputTokens)}
            detail={`${formatInteger(data.summary.uncachedInputTokens)} 未缓存`}
          />
          <MetricCard
            label="输出 Tokens"
            value={formatInteger(data.summary.outputTokens)}
            detail={`${formatInteger(data.summary.reasoningTokens)} 推理`}
          />
          <MetricCard
            label="Prompt Cache 读取率"
            value={formatPercent(data.summary.cacheReadRatio)}
            detail={cacheWriteDetail(data.summary)}
          />
          <MetricCard
            label="API 请求"
            value={formatInteger(data.summary.requestCount)}
            detail={`${formatPercent(data.summary.providerUsageCoverage)} usage 覆盖`}
          />
          <MetricCard
            label="平均端到端延迟"
            value={formatDuration(data.summary.averageLatencyMs)}
            detail={`P95 ${formatDuration(data.summary.p95LatencyMs)}`}
          />
        </section>

        <div className="usage-detail-grid">
          <Panel className="usage-detail-panel" title="API 与推理链路">
            <MetricList
              items={[
                ["平均 TTFT", formatDuration(data.summary.averageTtftMs)],
                [
                  "端到端输出速率",
                  formatRate(data.summary.outputTokensPerSecond),
                ],
                ["重试次数", formatInteger(data.summary.retryCount)],
                ["重试率", formatPercent(data.summary.retryRate)],
                [
                  "供应商 usage 覆盖",
                  `${formatInteger(data.summary.providerUsageReportedRequestCount)} / ${formatInteger(data.summary.successfulRequestCount)} 成功请求`,
                ],
                [
                  "校准后估算误差 P95",
                  formatPercent(data.summary.estimateErrorP95),
                ],
                [
                  "原始估算误差 P95",
                  formatPercent(data.summary.rawEstimateErrorP95),
                ],
                [
                  "当前校准系数",
                  formatFactor(data.summary.estimateCalibrationFactor),
                ],
                ["错误事件", formatInteger(data.summary.errorEventCount)],
                ["失败请求", formatInteger(data.summary.failedRequestCount)],
              ]}
            />
          </Panel>
          <Panel className="usage-detail-panel" title="缓存与 Token 构成">
            <MetricList
              items={[
                [
                  "未缓存输入（已报告）",
                  formatInteger(uncachedInput(data.summary)),
                ],
                [
                  "缓存读取 Tokens",
                  formatInteger(data.summary.cachedInputTokens),
                ],
                [
                  "缓存写入 Tokens",
                  formatInteger(data.summary.cacheWriteTokens),
                ],
                ["完整复用中断", formatInteger(data.summary.cacheBreakCount)],
                [
                  "部分命中下降",
                  formatInteger(data.summary.cacheDegradationCount),
                ],
                ["推理 Tokens", formatInteger(data.summary.reasoningTokens)],
                ["可见输出 Tokens", formatInteger(visibleOutput(data.summary))],
                ["缓存字段覆盖", cacheCoverageLabel(data.calls)],
                [
                  "未缓存 Tokens / 成功任务",
                  formatInteger(data.summary.uncachedTokensPerSuccessfulTask),
                ],
              ]}
            />
          </Panel>
          <Panel className="usage-detail-panel" title="Harness 与工具">
            <MetricList
              items={[
                ["工具调用", formatInteger(data.summary.toolCallCount)],
                ["工具错误", formatInteger(data.summary.toolErrorCount)],
                [
                  "平均工具耗时",
                  formatDuration(data.summary.averageToolDurationMs),
                ],
                [
                  "成功请求",
                  formatInteger(data.summary.successfulRequestCount),
                ],
                ["运行中请求", formatInteger(data.summary.runningRequestCount)],
                ["成功任务", formatInteger(data.summary.successfulTurnCount)],
                ["失败任务", formatInteger(data.summary.failedTurnCount)],
                ["模型", distinctModels(data.calls)],
              ]}
            />
          </Panel>
        </div>

        <Panel
          className="usage-token-breakdown-panel"
          title="输入 Token 瀑布（本地估算）"
          actions={
            <Badge variant="neutral">
              {formatInteger(data.summary.tokenBreakdown.total)} Tokens
            </Badge>
          }
        >
          <p className="usage-token-breakdown-help">
            按请求实际组装的上下文模块归因；模块合计是原始估算，后续轮次会用同一任务中
            Provider 已返回 usage 的中位数校准调用前预算。两者都不替代实际
            usage。
          </p>
          <TokenBreakdownTable summary={data.summary} />
        </Panel>

        <Panel className="usage-waste-panel" title="可归因浪费与附加成本">
          <MetricList
            items={[
              [
                "重试输入风险（估算）",
                `${formatInteger(data.summary.estimatedRetryInputTokens)} Tokens`,
              ],
              [
                "Provider 兼容回退",
                formatInteger(data.summary.compatibilityRetryCount),
              ],
              [
                "无效工具循环中止",
                formatInteger(data.summary.invalidToolLoopCount),
              ],
              [
                "终态守卫驳回",
                formatInteger(data.summary.finalizationGuardRejectCount),
              ],
              [
                "无进展重复调用信号",
                formatInteger(data.summary.noProgressSignalCount),
              ],
              ["完全重复计划", formatInteger(data.summary.duplicatePlanCount)],
              [
                "上下文压缩调用",
                `${formatInteger(data.summary.compactionRequestCount)} 次 · ${formatInteger(data.summary.compactionTokens)} Tokens`,
              ],
              ["成本 / 成功任务", "—（需 Provider 账单）"],
            ]}
          />
        </Panel>

        <Panel
          className="usage-cache-break-panel"
          title="缓存复用断点"
          actions={
            <Badge
              variant={data.cacheBreaks.length > 0 ? "warning" : "neutral"}
            >
              {data.cacheBreaks.length} 个事件
            </Badge>
          }
        >
          <p className="usage-cache-break-help">
            API 只报告命中的缓存
            Token；下列位置由相邻请求的缓存结果、配置与上下文 hash
            推断，并标注置信度。
          </p>
          {data.cacheBreaks.length === 0 ? (
            <div className="usage-table-state">
              <CircleCheck size={20} aria-hidden="true" />
              <p>目前没有检测到缓存复用下降。</p>
              <span>
                至少需要两次返回缓存 usage 的可比较请求，才能定位复用断点。
              </span>
            </div>
          ) : (
            <div className="usage-cache-break-list">
              {data.cacheBreaks.map((call) => (
                <CacheBreakRecord call={call} key={call.id} />
              ))}
            </div>
          )}
        </Panel>

        <Panel
          className="usage-call-panel"
          title="API 调用明细"
          actions={<Badge variant="neutral">{data.calls.length} 次请求</Badge>}
        >
          {isLoading && data.calls.length === 0 ? (
            <div className="usage-table-state" role="status">
              正在加载使用日志…
            </div>
          ) : data.calls.length === 0 ? (
            <div className="usage-table-state">
              <Activity size={20} aria-hidden="true" />
              <p>这个会话还没有可用的 API 使用记录。</p>
              <span>
                完成一次模型调用后，这里会显示 API 返回的 usage
                和本地观测到的链路指标。
              </span>
            </div>
          ) : (
            <div className="usage-table-wrap">
              <table className="usage-call-table">
                <thead>
                  <tr>
                    <th scope="col">时间</th>
                    <th scope="col">模型 / 轮次</th>
                    <th scope="col" className="usage-number-cell">
                      总 Tokens
                    </th>
                    <th scope="col" className="usage-number-cell">
                      输入 / 输出
                    </th>
                    <th scope="col" className="usage-number-cell">
                      本地估算 / 误差
                    </th>
                    <th scope="col" className="usage-number-cell">
                      缓存读取
                    </th>
                    <th scope="col" className="usage-number-cell">
                      推理
                    </th>
                    <th scope="col" className="usage-number-cell">
                      TTFT
                    </th>
                    <th scope="col" className="usage-number-cell">
                      延迟
                    </th>
                    <th scope="col">状态</th>
                  </tr>
                </thead>
                <tbody>
                  {data.calls.map((call) => (
                    <UsageCallRow call={call} key={call.id} />
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </Panel>

        <p className="usage-log-footnote">
          Token 与缓存字段来自模型 API 的
          usage；TTFT、端到端延迟、工具耗时和重试由 OpenTopia
          事件时间计算。Prompt Cache 读取率 = cached input tokens / input
          tokens。缓存读取没有从逻辑 Token
          中扣除；重试输入风险和模块瀑布均为本地估算。 成本字段在没有 Provider
          账单或带版本价格表时保持为空。托管 API 不提供 GPU 利用率、KV block
          占用/逐出或服务端真实排队时间。缓存断点位置是 harness
          根据相邻请求前缀推断的；无法区分缓存过期、服务端逐出或路由漂移时会明确标为低置信。
        </p>
      </div>
    </div>
  );
}

function CacheBreakRecord({ call }: { call: UsageCall }) {
  const diagnostic = call.cacheReuse;
  return (
    <article className="usage-cache-break-item" data-state={diagnostic.state}>
      <div className="usage-cache-break-heading">
        <div>
          <TriangleAlert size={16} aria-hidden="true" />
          <strong>{cacheBreakTitle(diagnostic)}</strong>
        </div>
        <div>
          <Badge variant={cacheBreakBadgeVariant(diagnostic)}>
            {diagnostic.state === "broken" ? "复用中断" : "命中下降"}
          </Badge>
          <time dateTime={call.startedAt}>
            {formatDateTime(call.startedAt)}
          </time>
        </div>
      </div>
      <p>{cacheBreakExplanation(diagnostic)}</p>
      <dl className="usage-cache-break-meta">
        <div>
          <dt>缓存读取</dt>
          <dd>
            {formatInteger(diagnostic.previousCachedInputTokens)} →{" "}
            {formatInteger(diagnostic.currentCachedInputTokens)}
          </dd>
        </div>
        <div>
          <dt>估算损失</dt>
          <dd>{formatInteger(diagnostic.lostCachedTokens)} Tokens</dd>
        </div>
        <div>
          <dt>前缀位置</dt>
          <dd>{cacheBreakOffset(diagnostic)}</dd>
        </div>
        <div>
          <dt>置信度</dt>
          <dd>{cacheConfidenceLabel(diagnostic.confidence)}</dd>
        </div>
      </dl>
    </article>
  );
}

function cacheBreakBadgeVariant(
  diagnostic: CacheReuseDiagnostic,
): BadgeVariant {
  return diagnostic.state === "broken" ? "danger" : "warning";
}

function cacheBreakTitle(diagnostic: CacheReuseDiagnostic): string {
  const point = diagnostic.breakpoint;
  switch (diagnostic.reason) {
    case "content_changed":
      return point
        ? `最早变化：${contextKindLabel(point.kind)} · ${point.source}`
        : "输入前缀内容发生变化";
    case "tool_catalog_changed":
      return "工具定义或顺序发生变化";
    case "system_prompt_changed":
      return "系统提示发生变化";
    case "cache_key_changed":
      return "Prompt Cache Key 发生变化";
    case "model_changed":
      return "模型发生切换";
    case "provider_changed":
      return "API Provider 发生切换";
    case "input_below_minimum":
      return "输入低于 OpenAI 缓存最低长度";
    case "stateful_context":
      return "状态游标请求无法直接比较完整前缀";
    case "operational_miss":
      return "未发现提前变化的输入前缀";
    default:
      return "缓存读取下降";
  }
}

function cacheBreakExplanation(diagnostic: CacheReuseDiagnostic): string {
  const point = diagnostic.breakpoint;
  switch (diagnostic.reason) {
    case "content_changed":
      return point
        ? `${contextChangeLabel(point.change)}；缓存范围为${cacheScopeLabel(point.cacheScope)}。该位置是 token_estimate 推断，不是 API 返回的逐 Token 断点。`
        : "相邻请求的上下文 hash 不一致，但没有足够的上下文项用于精确定位。";
    case "tool_catalog_changed":
      return "工具名称、描述、输入 Schema 或排列顺序的变化都可能使缓存前缀失配。";
    case "system_prompt_changed":
      return "系统或开发者指令位于提示词前部，变化通常会影响其后的整段缓存复用。";
    case "cache_key_changed":
      return "相邻请求没有使用相同的 prompt_cache_key，缓存路由与匹配不再可直接复用。";
    case "model_changed":
      return "不同模型的 KV Cache 不能作为同一条可比较缓存链处理。";
    case "provider_changed":
      return "请求切换了 Provider，服务端缓存不共享。";
    case "input_below_minimum":
      return "该请求少于 1,024 个输入 Token，OpenAI 的 Prompt Cache 不会产生读取命中。";
    case "stateful_context":
      return "请求使用 previous_response_id 复用服务端状态；若已记录内容没有变化，缓存下降也可能来自状态游标的计量差异。";
    case "operational_miss":
      return "已记录的公共前缀未提前变化，更可能是缓存过期、服务端逐出、路由漂移或负载分片造成。";
    default:
      return "现有事件不足以确定具体内容位置。";
  }
}

function cacheBreakOffset(diagnostic: CacheReuseDiagnostic): string {
  const offset = diagnostic.breakpoint?.tokenOffsetEstimate;
  return offset === null || offset === undefined
    ? "无法定位"
    : `约 ${formatInteger(offset)} Tokens 后`;
}

function contextKindLabel(kind: string): string {
  const labels: Record<string, string> = {
    base_instructions: "基础指令",
    developer_instructions: "开发者指令",
    repository_instructions: "仓库指令",
    environment: "环境上下文",
    world_state: "世界状态",
    skill: "Skill",
    summary: "上下文摘要",
    checkpoint: "上下文检查点",
    conversation: "会话历史",
    user: "用户输入",
    tool_call: "工具调用",
    tool_result: "工具结果",
  };
  return labels[kind] ?? kind;
}

function contextChangeLabel(
  change: NonNullable<CacheReuseDiagnostic["breakpoint"]>["change"],
): string {
  if (change === "inserted") return "该上下文项被插入到公共前缀中";
  if (change === "removed") return "该上下文项从公共前缀中移除";
  return "该上下文项的内容 hash 发生变化";
}

function cacheScopeLabel(
  scope: NonNullable<CacheReuseDiagnostic["breakpoint"]>["cacheScope"],
): string {
  const labels = {
    stable: "稳定级",
    thread: "会话级",
    turn: "回合级",
    round: "推理轮次级",
    none: "未缓存级",
  } as const;
  return scope ? labels[scope] : "未知级";
}

function cacheConfidenceLabel(
  confidence: CacheReuseDiagnostic["confidence"],
): string {
  if (confidence === "high") return "高 · 配置/API 证据";
  if (confidence === "medium") return "中 · 上下文 hash 推断";
  return "低 · 服务端状态不可见";
}

function MetricCard({
  label,
  value,
  detail,
}: {
  label: string;
  value: string;
  detail: string;
}) {
  return (
    <article className="usage-metric-card">
      <span>{label}</span>
      <strong>{value}</strong>
      <small>{detail}</small>
    </article>
  );
}

function MetricList({ items }: { items: Array<[string, string]> }) {
  return (
    <dl className="usage-metric-list">
      {items.map(([label, value]) => (
        <div key={label}>
          <dt>{label}</dt>
          <dd>{value}</dd>
        </div>
      ))}
    </dl>
  );
}

function TokenBreakdownTable({ summary }: { summary: UsageSummary }) {
  const breakdown = summary.tokenBreakdown;
  const allRows: Array<[string, number]> = [
    ["基础指令", breakdown.baseInstructions],
    ["开发者指令", breakdown.developerInstructions],
    ["仓库指令", breakdown.repositoryInstructions],
    ["运行时环境", breakdown.runtimeContext],
    ["Skills", breakdown.skillInstructions],
    ["上下文摘要", breakdown.summaries],
    ["检查点", breakdown.checkpoints],
    ["会话历史", breakdown.conversation],
    ["当前用户输入", breakdown.currentUser],
    ["工具调用", breakdown.toolCalls],
    ["工具结果", breakdown.toolResults],
    ["工具 / 输出 Schema", breakdown.toolSchemas],
    ["Provider 状态对象", breakdown.providerState],
    ["其他", breakdown.other],
  ];
  const rows = allRows.filter(([, tokens]) => tokens > 0);

  if (rows.length === 0) {
    return (
      <div className="usage-table-state">
        <span>新请求完成后会记录可审计的模块级 Token 构成。</span>
      </div>
    );
  }

  return (
    <div className="usage-table-wrap">
      <table className="usage-token-breakdown-table">
        <thead>
          <tr>
            <th scope="col">输入模块</th>
            <th scope="col" className="usage-number-cell">
              估算 Tokens
            </th>
            <th scope="col" className="usage-number-cell">
              占比
            </th>
          </tr>
        </thead>
        <tbody>
          {rows.map(([label, tokens]) => (
            <tr key={label}>
              <th scope="row">{label}</th>
              <td className="usage-number-cell">{formatInteger(tokens)}</td>
              <td className="usage-number-cell">
                {formatPercent(
                  breakdown.total > 0 ? tokens / breakdown.total : null,
                )}
              </td>
            </tr>
          ))}
        </tbody>
        <tfoot>
          <tr>
            <th scope="row">合计</th>
            <td className="usage-number-cell">
              {formatInteger(breakdown.total)}
            </td>
            <td className="usage-number-cell">100%</td>
          </tr>
        </tfoot>
      </table>
    </div>
  );
}

function UsageCallRow({ call }: { call: UsageCall }) {
  const cacheRatio =
    call.inputTokens > 0 ? call.cachedInputTokens / call.inputTokens : null;
  return (
    <tr>
      <td>
        <time dateTime={call.startedAt}>{formatDateTime(call.startedAt)}</time>
      </td>
      <td>
        <span className="usage-call-model" title={call.endpoint}>
          {call.model ?? call.adapter}
        </span>
        <small>
          {callPurposeLabel(call.purpose)} · Round {call.round}
          {call.providerId ? ` · ${call.providerId}` : ""}
        </small>
      </td>
      <td className="usage-number-cell">{formatInteger(call.totalTokens)}</td>
      <td className="usage-number-cell">
        {formatInteger(call.inputTokens)} / {formatInteger(call.outputTokens)}
      </td>
      <td className="usage-number-cell">
        {formatInteger(call.contextTokenEstimate)}
        <small>{formatPercent(call.estimateErrorRatio)}</small>
      </td>
      <td className="usage-number-cell">
        {call.cacheReadTokensReported
          ? formatInteger(call.cachedInputTokens)
          : "—"}
        {call.cacheReadTokensReported ? (
          <small>{formatPercent(cacheRatio)}</small>
        ) : null}
      </td>
      <td className="usage-number-cell">
        {formatInteger(call.reasoningTokens)}
      </td>
      <td className="usage-number-cell">{formatDuration(call.ttftMs)}</td>
      <td className="usage-number-cell">{formatDuration(call.durationMs)}</td>
      <td>
        <Badge variant={statusBadgeVariant(call)}>{statusLabel(call)}</Badge>
        {call.retryCount > 0 ? <small>重试 {call.retryCount}</small> : null}
      </td>
    </tr>
  );
}

function callPurposeLabel(purpose: UsageCall["purpose"]): string {
  const labels: Record<UsageCall["purpose"], string> = {
    agent_round: "Agent",
    context_compaction: "上下文压缩",
    guardian_review: "Guardian",
    title_generation: "标题生成",
    other: "其他",
  };
  return labels[purpose];
}

function statusBadgeVariant(call: UsageCall): BadgeVariant {
  if (call.status === "failed") return "danger";
  if (call.status === "running") return "info";
  return "success";
}

function statusLabel(call: UsageCall): string {
  if (call.status === "failed") {
    return call.statusCode ? `失败 ${call.statusCode}` : "失败";
  }
  if (call.status === "running") return "进行中";
  return call.statusCode ? `成功 ${call.statusCode}` : "成功";
}

function uncachedInput(summary: UsageSummary): number | null {
  if (summary.cacheReadReportedRequestCount === 0) return null;
  return Math.max(0, summary.cacheReadInputTokens - summary.cachedInputTokens);
}

function visibleOutput(summary: UsageSummary): number {
  return Math.max(0, summary.outputTokens - summary.reasoningTokens);
}

function cacheCoverageLabel(calls: UsageCall[]): string {
  if (calls.length === 0) return "—";
  const reported = calls.filter(
    (call) => call.cacheReadTokensReported || call.cacheWriteTokensReported,
  ).length;
  return `${formatInteger(reported)} / ${formatInteger(calls.length)} 请求`;
}

function cacheWriteDetail(summary: UsageSummary): string {
  return summary.cacheWriteReportedRequestCount > 0
    ? `${formatInteger(summary.cacheWriteTokens)} 写入缓存`
    : "缓存写入字段未返回";
}

function distinctModels(calls: UsageCall[]): string {
  const models = new Set(calls.map((call) => call.model).filter(Boolean));
  if (models.size === 0) return "—";
  if (models.size === 1) return [...models][0] ?? "—";
  return `${models.size} 个模型`;
}

function formatInteger(value: number | null): string {
  if (value === null || !Number.isFinite(value)) return "—";
  return new Intl.NumberFormat("zh-CN", { maximumFractionDigits: 0 }).format(
    value,
  );
}

function formatPercent(value: number | null): string {
  if (value === null || !Number.isFinite(value)) return "—";
  return new Intl.NumberFormat("zh-CN", {
    style: "percent",
    maximumFractionDigits: 1,
  }).format(value);
}

function formatDuration(value: number | null): string {
  if (value === null || !Number.isFinite(value)) return "—";
  if (value < 1_000) return `${Math.round(value)} ms`;
  return `${(value / 1_000).toFixed(value < 10_000 ? 2 : 1)} s`;
}

function formatRate(value: number | null): string {
  if (value === null || !Number.isFinite(value)) return "—";
  return `${value.toFixed(1)} tok/s`;
}

function formatFactor(value: number | null): string {
  if (value === null || !Number.isFinite(value)) return "—";
  return `×${value.toFixed(2)}`;
}

function formatDateTime(value: string): string {
  const date = new Date(value);
  if (!Number.isFinite(date.getTime())) return value;
  return new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  }).format(date);
}
