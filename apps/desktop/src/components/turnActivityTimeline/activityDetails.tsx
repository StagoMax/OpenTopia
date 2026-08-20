import { useState, type ReactNode } from "react";
import {
  Activity,
  AlertCircle,
  ChevronDown,
  ChevronRight,
  CircleSlash,
  CloudOff,
  Clock3,
  FileWarning,
  Loader2,
  Minimize2,
  ShieldCheck,
  ShieldX,
  X,
} from "lucide-react";
import type { ContextCompactionDetails, WorkForm } from "../../types";
import { redactText, type ToolActivityChip } from "../../toolActivity";
import {
  guardianActivityPresentation,
  type GuardianActivityIcon,
  type GuardianActivityMetric,
} from "../../guardianActivity";
import { MarkdownContent } from "../MarkdownContent";
import { ActivityCallCard } from "../ToolActivityCard";
import type { ActivityEntry, WorkItemTiming } from "./model";
import { formatActivityTiming, formatWorkItemTiming } from "./timing";

export function NarrativeActivity({
  onOpenMarkdownLink,
  streaming,
  text,
  traceThreadId,
  traceTurnId,
}: {
  onOpenMarkdownLink?(href: string): void;
  streaming: boolean;
  text: string;
  traceThreadId?: string;
  traceTurnId?: string | null;
}) {
  return (
    <div
      className="activity-narrative"
      data-kind="commentary"
      aria-label="处理说明"
    >
      <MarkdownContent
        className="activity-narrative-markdown"
        onOpenLink={onOpenMarkdownLink}
        renderTrace={
          traceThreadId
            ? {
                channel: "commentary",
                threadId: traceThreadId,
                turnId: traceTurnId,
              }
            : undefined
        }
        streaming={streaming}
        text={redactText(text)}
      />
    </div>
  );
}

export function WorkFormActivity({
  form,
  itemTimings,
  startedAt,
  finishedAt,
  defaultExpanded,
  isActive,
  now,
}: {
  form: WorkForm;
  itemTimings: WorkItemTiming[];
  startedAt: string;
  finishedAt?: string;
  defaultExpanded: boolean;
  isActive: boolean;
  now: number;
}) {
  const resolved = form.items.filter((item) =>
    ["completed", "deferred", "blocked", "cancelled"].includes(item.status),
  ).length;
  const running = form.items.some((item) => item.status === "in_progress");
  const actionable = form.items.some(
    (item) => item.status === "pending" || item.status === "in_progress",
  );
  const completedIds = new Set(
    form.items
      .filter((item) => item.status === "completed")
      .map((item) => item.id),
  );
  let currentStepIndex = form.items.findIndex(
    (item) => item.status === "in_progress",
  );
  if (currentStepIndex < 0) {
    currentStepIndex = form.items.findIndex(
      (item) =>
        item.status === "pending" &&
        item.dependsOn.every((dependency) => completedIds.has(dependency)),
    );
  }
  const timing = formatActivityTiming(
    startedAt,
    finishedAt,
    running && isActive,
    now,
  );
  const [expanded, setExpanded] = useState(defaultExpanded || actionable);
  return (
    <div
      className="activity-group"
      data-state={actionable ? "running" : "complete"}
    >
      <button
        className="activity-group-header"
        type="button"
        aria-expanded={expanded}
        onClick={() => setExpanded((current) => !current)}
      >
        <span className="activity-group-icon" aria-hidden="true">
          <Activity size={13} />
        </span>
        <span>执行计划</span>
        <small className="activity-group-count">
          {currentStepIndex >= 0
            ? `第 ${currentStepIndex + 1}/${form.items.length} 步`
            : `${resolved}/${form.items.length} 已处理`}
          {timing ? ` · ${timing}` : ""}
        </small>
        {expanded ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
      </button>
      {expanded && (
        <div className="activity-plan">
          {form.changeReason && <p>{form.changeReason}</p>}
          <ol>
            {form.items.map((item, index) => {
              const title = item.title || item.id;
              const stepTiming = formatWorkItemTiming(
                itemTimings[index],
                item.status,
                isActive,
                now,
              );
              return (
                <li
                  key={item.id || `${index}-${title}`}
                  data-status={item.status}
                >
                  <span aria-hidden="true">
                    {item.status === "completed" ? (
                      <span className="activity-plan-dot is-complete" />
                    ) : item.status === "in_progress" ? (
                      <ActivityFlow compact />
                    ) : item.status === "blocked" ? (
                      <AlertCircle size={12} />
                    ) : item.status === "cancelled" ? (
                      <X size={12} />
                    ) : item.status === "deferred" ? (
                      <Clock3 size={12} />
                    ) : (
                      <span className="activity-plan-dot" />
                    )}
                  </span>
                  <span className="activity-plan-step-body">
                    <span>
                      {title}
                      {stepTiming ? ` · ${stepTiming}` : ""}
                    </span>
                    {item.note && <small>{item.note}</small>}
                  </span>
                </li>
              );
            })}
          </ol>
        </div>
      )}
    </div>
  );
}

export function ActivityNotice({
  className,
  icon,
  title,
  detail,
  metrics = [],
  tone = "neutral",
}: {
  className?: string;
  icon: ReactNode;
  title: string;
  detail?: string;
  metrics?: GuardianActivityMetric[];
  tone?: "neutral" | "waiting" | "success" | "error";
}) {
  return (
    <div
      className={`activity-notice${className ? ` ${className}` : ""}`}
      data-tone={tone}
    >
      <span aria-hidden="true">{icon}</span>
      <div>
        <strong>{title}</strong>
        {detail && <p>{detail}</p>}
        {metrics.length > 0 && (
          <dl className="activity-notice-metrics">
            {metrics.map((metric) => (
              <div key={metric.label}>
                <dt>{metric.label}</dt>
                <dd>{metric.value}</dd>
              </div>
            ))}
          </dl>
        )}
      </div>
    </div>
  );
}

export function GuardianReviewActivity({
  entry,
  now,
}: {
  entry: Extract<ActivityEntry, { kind: "guardian-review" }>;
  now: number;
}) {
  const timing = formatActivityTiming(
    entry.startedAt,
    entry.finishedAt,
    !entry.completed,
    now,
  );
  if (!entry.completed) {
    return (
      <ActivityNotice
        icon={<Loader2 className="spin" size={13} />}
        tone="waiting"
        title="正在自动审批"
        detail={timing ? `已用时 ${timing}` : "正在评估待执行操作。"}
      />
    );
  }

  const presentation = guardianActivityPresentation(entry.completed);
  const detail = [presentation.rationale, timing ? `耗时 ${timing}` : null]
    .filter((value): value is string => Boolean(value))
    .join("\n");
  return (
    <ActivityNotice
      icon={guardianActivityIcon(presentation.icon)}
      tone={presentation.tone}
      title={presentation.title}
      detail={detail}
      metrics={presentation.metrics}
    />
  );
}

function guardianActivityIcon(kind: GuardianActivityIcon): ReactNode {
  switch (kind) {
    case "approved":
      return <ShieldCheck size={13} />;
    case "waiting":
      return <Clock3 size={13} />;
    case "denied":
      return <ShieldX size={13} />;
    case "unavailable":
      return <CloudOff size={13} />;
    case "invalid":
      return <FileWarning size={13} />;
    case "aborted":
      return <CircleSlash size={13} />;
  }
}

export function ContextCompactionActivity({
  entry,
  now,
}: {
  entry: Extract<ActivityEntry, { kind: "context-compaction" }>;
  now: number;
}) {
  const running = !entry.finishedAt;
  const failed = Boolean(entry.error);
  const metrics = entry.details?.metrics;
  const messageCount =
    entry.details?.coverage.throughMessageCount ?? entry.messageCount;
  const inputTokens = metrics?.inputTokens || entry.inputTokenEstimate;
  const checkpointTokens =
    metrics?.checkpointTokens || entry.checkpointTokenEstimate;
  const chips: ToolActivityChip[] = [];
  if (entry.modelRequestCount > 1) {
    chips.push({ label: `${entry.modelRequestCount} 次模型调用` });
  }
  if (messageCount) chips.push({ label: `${messageCount} 条消息` });
  if (metrics?.tokenReductionPercent) {
    chips.push({
      label: `减少 ${metrics.tokenReductionPercent}%`,
      tone: "success",
    });
  }
  if (failed) chips.push({ label: "失败", tone: "danger" });
  const timing = formatActivityTiming(
    entry.createdAt,
    entry.finishedAt,
    running,
    now,
    metrics?.latencyMs ? metrics.latencyMs : undefined,
  );
  const fields = [
    {
      label: "压缩方式",
      value: contextCompactionModeLabel(entry.details?.mode),
    },
    messageCount
      ? {
          label: "覆盖范围",
          value: `${messageCount.toLocaleString()} 条消息${
            entry.details?.coverage.throughSeq
              ? ` · 至事件 #${entry.details.coverage.throughSeq.toLocaleString()}`
              : ""
          }`,
        }
      : null,
    inputTokens
      ? { label: "输入 Token", value: inputTokens.toLocaleString() }
      : null,
    checkpointTokens
      ? {
          label: "检查点 Token",
          value: checkpointTokens.toLocaleString(),
        }
      : null,
    metrics?.factRetentionPercent
      ? {
          label: "事实保留",
          value: `${metrics.factRetentionPercent}%`,
        }
      : null,
  ].filter((field): field is { label: string; value: string } =>
    Boolean(field),
  );

  return (
    <ActivityCallCard
      identity={entry.requestId}
      state={failed ? "error" : running ? "running" : "complete"}
      kind="context-compaction"
      icon={<Minimize2 size={14} />}
      title={
        failed ? "上下文压缩失败" : running ? "正在压缩上下文" : "已压缩上下文"
      }
      detail={contextCompactionModeLabel(entry.details?.mode)}
      chips={chips}
      timing={timing}
      defaultExpanded={failed}
    >
      {running ? (
        <div className="tool-panel is-pending" role="status">
          正在读取历史并生成上下文检查点…
        </div>
      ) : failed ? (
        <div className="tool-panel" data-panel="text">
          <div className="tool-panel-scroll">
            <p className="context-compaction-summary" data-tone="error">
              {redactText(entry.error ?? "上下文压缩未完成。")}
            </p>
          </div>
        </div>
      ) : (
        <div className="tool-panel" data-panel="fields">
          <dl className="tool-panel-fields">
            {fields.map((field) => (
              <div key={field.label}>
                <dt>{field.label}</dt>
                <dd>{field.value}</dd>
              </div>
            ))}
          </dl>
          {entry.summary && (
            <div className="tool-panel-scroll">
              <p className="context-compaction-summary">
                {redactText(entry.summary)}
              </p>
            </div>
          )}
        </div>
      )}
    </ActivityCallCard>
  );
}

function contextCompactionModeLabel(mode?: ContextCompactionDetails["mode"]) {
  if (mode === "native_provider") return "提供商原生压缩";
  if (mode === "structured_local") return "本地检查点";
  if (mode === "manual") return "手动检查点";
  if (mode === "legacy_text") return "旧版摘要";
  return "上下文检查点";
}

export function ActivityResultIcon({ running }: { running: boolean }) {
  if (running) return <ActivityFlow compact />;
  return <span className="activity-result-spacer" aria-hidden="true" />;
}

export function ActivityFlow({ compact = false }: { compact?: boolean }) {
  return (
    <span
      className={`activity-flow${compact ? " is-compact" : ""}`}
      aria-hidden="true"
    />
  );
}
