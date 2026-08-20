import { useMemo, useState } from "react";

import type { UsageCall, UsageSummary } from "../usageLogs";
import { modelBreakdownGroups } from "../usageTokenBreakdownModels";
import { Badge, Panel, SegmentedControl } from "./ui";
import {
  UsageTokenBreakdownModelCard,
  UsageTokenBreakdownTable,
} from "./UsageTokenBreakdownDetails";

type BreakdownScope = "cumulative" | "latest" | "by_model";

export function UsageTokenBreakdown({
  calls,
  summary,
}: {
  calls: UsageCall[];
  summary: UsageSummary;
}) {
  const latestCall = calls.find((call) => call.inputBreakdown !== null) ?? null;
  const modelGroups = useMemo(() => modelBreakdownGroups(calls), [calls]);
  const [scope, setScope] = useState<BreakdownScope>("cumulative");
  const effectiveScope =
    scope === "by_model" && modelGroups.length < 2 ? "cumulative" : scope;
  const breakdown =
    effectiveScope === "latest" && latestCall?.inputBreakdown
      ? latestCall.inputBreakdown
      : summary.tokenBreakdown;
  const scopedCalls =
    effectiveScope === "latest"
      ? latestCall
        ? [latestCall]
        : []
      : calls.filter((call) => call.inputBreakdown !== null);
  const reportedCalls = scopedCalls.filter(
    (call) => call.providerUsageReported,
  );
  const actualInputTokens = reportedCalls.reduce(
    (total, call) => total + call.inputTokens,
    0,
  );
  const actualIsComplete =
    scopedCalls.length > 0 && reportedCalls.length === scopedCalls.length;
  const difference = actualIsComplete
    ? actualInputTokens - breakdown.total
    : null;

  return (
    <Panel
      className="usage-token-breakdown-panel"
      title="实际输入与构成归因"
      actions={
        <div className="usage-token-breakdown-actions">
          <SegmentedControl<BreakdownScope>
            value={effectiveScope}
            options={[
              { value: "cumulative", label: "请求累计" },
              {
                value: "latest",
                label: "最新请求",
                disabled: latestCall === null,
              },
              {
                value: "by_model",
                label: "按模型",
                disabled: modelGroups.length < 2,
              },
            ]}
            onChange={(nextScope) => setScope(nextScope as BreakdownScope)}
            label="Token 构成统计范围"
          />
          <Badge variant="neutral">
            本地归因 {formatInteger(breakdown.total)}
          </Badge>
        </div>
      }
    >
      <p className="usage-token-breakdown-help">
        Provider usage 给出请求级实际输入总量；下方子层由 OpenTopia
        按发送前的请求结构在本地归因，因此子层仍是估算。input tokens
        表示一次模型请求读取的完整上下文，不是会话唯一 Token
        数。“请求累计”会把同一段历史在多个 Round
        中的重复读取逐次相加。工具调用和结果只统计当前 Turn
        截至该请求发送前已发生的内容；助手输出在后续被保留为历史或 Provider
        续接项时，才成为后续请求的输入。 “Provider
        不透明续接状态”只保留加密推理、压缩项和 Chat 调用关联等普通
        user/assistant 历史无法重建的内容。
      </p>

      {effectiveScope === "by_model" ? (
        <div
          className="usage-model-breakdown-list"
          aria-label="按模型拆分的输入 Token 构成"
        >
          {modelGroups.map((group) => (
            <UsageTokenBreakdownModelCard group={group} key={group.key} />
          ))}
        </div>
      ) : (
        <>
          <div
            className="usage-token-breakdown-summary"
            aria-label="输入 Token 对照"
          >
            <div>
              <span>Provider 实际输入</span>
              <strong>
                {reportedCalls.length > 0
                  ? formatInteger(actualInputTokens)
                  : "—"}
              </strong>
              <small>
                usage 覆盖 {reportedCalls.length} / {scopedCalls.length} 个请求
              </small>
            </div>
            <div>
              <span>本地构成归因</span>
              <strong>{formatInteger(breakdown.total)}</strong>
              <small>用于解释组成，不作为账单依据</small>
            </div>
            <div>
              <span>完整覆盖时差值</span>
              <strong>
                {difference === null ? "—" : formatSigned(difference)}
              </strong>
              <small>Provider 实际值 − 本地估算</small>
            </div>
          </div>

          <UsageTokenBreakdownTable breakdown={breakdown} />
        </>
      )}
    </Panel>
  );
}

function formatInteger(value: number | null): string {
  return value === null ? "—" : Math.round(value).toLocaleString("zh-CN");
}

function formatSigned(value: number): string {
  if (value === 0) return "0";
  return `${value > 0 ? "+" : "−"}${formatInteger(Math.abs(value))}`;
}
