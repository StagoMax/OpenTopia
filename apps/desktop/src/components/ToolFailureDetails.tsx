import { useMemo, useState } from "react";
import { ChevronDown, CircleCheck, TriangleAlert } from "lucide-react";

import { collectToolFailures, type ToolFailureDetail } from "../toolFailures";
import { buildToolActivity, redactText, truncateLine } from "../toolActivity";
import type { AgentEvent } from "../types";
import { Badge, Button, Panel } from "./ui";
import "./ToolFailureDetails.css";

type ToolFailureDetailsProps = {
  events: AgentEvent[];
  isLoading: boolean;
};

const failurePageSize = 50;
const dateTimeFormatter = new Intl.DateTimeFormat("zh-CN", {
  month: "2-digit",
  day: "2-digit",
  hour: "2-digit",
  minute: "2-digit",
  second: "2-digit",
  hour12: false,
});

export function ToolFailureDetails({
  events,
  isLoading,
}: ToolFailureDetailsProps) {
  const failures = useMemo(() => collectToolFailures(events), [events]);
  const [visibleCount, setVisibleCount] = useState(failurePageSize);
  const visibleFailures = failures.slice(0, visibleCount);
  const hiddenCount = Math.max(0, failures.length - visibleFailures.length);

  return (
    <Panel
      className="usage-tool-failure-panel"
      title="工具失败明细"
      actions={
        <Badge variant={failures.length > 0 ? "danger" : "neutral"}>
          {failures.length} 次失败
        </Badge>
      }
    >
      <p className="usage-tool-failure-help">
        汇总当前会话中失败的工具结果。展开记录可查看失败阶段、是否已执行、可重试性和完整错误链。
      </p>
      {isLoading && failures.length === 0 ? (
        <div className="usage-table-state" role="status">
          正在加载工具结果…
        </div>
      ) : failures.length === 0 ? (
        <div className="usage-table-state">
          <CircleCheck size={20} aria-hidden="true" />
          <p>当前会话没有工具调用失败。</p>
          <span>后续发生失败时，这里会保留工具、时间和具体原因。</span>
        </div>
      ) : (
        <>
          <div className="usage-tool-failure-list" aria-label="工具失败记录">
            {visibleFailures.map((failure) => (
              <ToolFailureRecord failure={failure} key={failure.eventId} />
            ))}
          </div>
          {hiddenCount > 0 ? (
            <div className="usage-tool-failure-more">
              <Button
                size="compact"
                variant="quiet"
                onClick={() =>
                  setVisibleCount((current) => current + failurePageSize)
                }
              >
                显示更多（剩余 {hiddenCount} 条）
              </Button>
            </div>
          ) : null}
        </>
      )}
    </Panel>
  );
}

function ToolFailureRecord({ failure }: { failure: ToolFailureDetail }) {
  const activity = failure.call ? buildToolActivity(failure.call) : null;
  const title = redactText(activity?.title ?? failure.toolName);
  const detail = activity?.detail ? redactText(activity.detail) : null;
  const message = redactText(failure.message);
  const causes = failure.causes.map(redactText);

  return (
    <details className="usage-tool-failure-item">
      <summary>
        <span className="usage-tool-failure-icon">
          <TriangleAlert size={16} aria-hidden="true" />
        </span>
        <span className="usage-tool-failure-summary">
          <span className="usage-tool-failure-heading">
            <strong>{title}</strong>
            {detail ? <small>{detail}</small> : null}
            <Badge variant="danger">{failure.code ?? "失败"}</Badge>
            <time dateTime={failure.createdAt}>
              {formatDateTime(failure.createdAt)}
            </time>
          </span>
          <span className="usage-tool-failure-preview">
            {truncateLine(message, 220)}
          </span>
        </span>
        <ChevronDown
          className="usage-tool-failure-chevron"
          size={16}
          aria-hidden="true"
        />
      </summary>

      <div className="usage-tool-failure-body">
        <dl className="usage-tool-failure-meta">
          <div>
            <dt>工具</dt>
            <dd>{failure.toolName}</dd>
          </div>
          <div>
            <dt>失败阶段</dt>
            <dd>{phaseLabel(failure.phase)}</dd>
          </div>
          <div>
            <dt>执行状态</dt>
            <dd>{executionLabel(failure.executed)}</dd>
          </div>
          <div>
            <dt>重试提示</dt>
            <dd>{retryLabel(failure.retryable)}</dd>
          </div>
        </dl>

        <section className="usage-tool-failure-reason">
          <h3>失败原因</h3>
          <pre>{message}</pre>
        </section>

        {causes.length > 0 ? (
          <section className="usage-tool-failure-causes">
            <h3>错误链</h3>
            <ol>
              {causes.map((cause, index) => (
                <li key={`${failure.eventId}-cause-${index}`}>{cause}</li>
              ))}
            </ol>
          </section>
        ) : null}

        <dl className="usage-tool-failure-identifiers">
          <div>
            <dt>Call ID</dt>
            <dd>{failure.callId}</dd>
          </div>
          {failure.turnId ? (
            <div>
              <dt>Turn ID</dt>
              <dd>{failure.turnId}</dd>
            </div>
          ) : null}
        </dl>
      </div>
    </details>
  );
}

function formatDateTime(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : dateTimeFormatter.format(date);
}

function phaseLabel(phase: string | null): string {
  switch (phase) {
    case "validation":
      return "参数校验";
    case "authorization":
      return "权限校验";
    case "preflight":
      return "执行准备";
    case "scheduling":
      return "任务调度";
    case "execution":
      return "工具执行";
    default:
      return phase ?? "未记录";
  }
}

function executionLabel(executed: boolean | null): string {
  if (executed === true) return "已开始执行";
  if (executed === false) return "执行前失败";
  return "未记录";
}

function retryLabel(retryable: boolean | null): string {
  if (retryable === true) return "可调整后重试";
  if (retryable === false) return "未标记为可重试";
  return "未记录";
}
