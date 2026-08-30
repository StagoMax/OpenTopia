import type { FlowRunStatus, WorkflowCheckpointStatus } from "../../types";
import type { BadgeVariant } from "../ui/Badge";
import { humanizeIdentifier } from "./enterpriseSidebarPresentation.ts";

export type RunStatusPresentation = {
  description: string;
  label: string;
  variant: BadgeVariant;
};

const RUN_STATUS_PRESENTATION: Record<FlowRunStatus, RunStatusPresentation> = {
  queued: {
    description: "已进入队列，正在等待可用的执行资源。",
    label: "排队中",
    variant: "neutral",
  },
  running: {
    description: "流程正在执行，页面会持续记录节点进度。",
    label: "运行中",
    variant: "info",
  },
  pause_requested: {
    description: "已请求暂停，当前步骤安全结束后会保存状态。",
    label: "暂停中",
    variant: "warning",
  },
  paused: {
    description: "运行状态已保存，可以从当前检查点继续。",
    label: "已暂停",
    variant: "warning",
  },
  waiting_approval: {
    description: "流程已停在审批点，等待人工确认后继续。",
    label: "等待审批",
    variant: "warning",
  },
  waiting_human: {
    description: "流程需要补充信息或人工处理后才能继续。",
    label: "等待处理",
    variant: "warning",
  },
  resuming: {
    description: "正在从最近的检查点恢复执行。",
    label: "恢复中",
    variant: "info",
  },
  succeeded: {
    description: "所有必要步骤均已完成，最终结果已经生成。",
    label: "运行成功",
    variant: "success",
  },
  failed: {
    description: "运行未能完成，请查看失败节点和错误信息。",
    label: "运行失败",
    variant: "danger",
  },
  cancel_requested: {
    description: "已请求取消，当前步骤安全结束后会停止运行。",
    label: "取消中",
    variant: "warning",
  },
  cancelled: {
    description: "运行已取消，已完成的检查点仍然保留。",
    label: "已取消",
    variant: "danger",
  },
};

export type PayloadFieldPresentation = {
  description: string | null;
  key: string;
  label: string;
  schema: unknown;
  value: unknown;
};

export function runStatusPresentation(
  status: FlowRunStatus,
): RunStatusPresentation {
  return RUN_STATUS_PRESENTATION[status];
}

export function checkpointStatusLabel(
  status: WorkflowCheckpointStatus,
): string {
  if (status === "running") return "保存中";
  if (status === "committed") return "已保存";
  if (status === "failed") return "保存失败";
  return "已取消";
}

export function payloadFields(
  payload: Record<string, unknown>,
  schema: unknown,
): PayloadFieldPresentation[] {
  const properties = schemaProperties(schema);
  const schemaKeys = Object.keys(properties).filter((key) =>
    Object.hasOwn(payload, key),
  );
  const extraKeys = Object.keys(payload).filter(
    (key) => !Object.hasOwn(properties, key),
  );

  return [...schemaKeys, ...extraKeys].map((key) => {
    const fieldSchema = properties[key];
    return {
      description: schemaText(fieldSchema, "description"),
      key,
      label: schemaText(fieldSchema, "title") ?? humanizeIdentifier(key),
      schema: fieldSchema,
      value: payload[key],
    };
  });
}

export function payloadItemSchema(schema: unknown): unknown {
  return isRecord(schema) ? schema.items : undefined;
}

export function formatDuration(
  startedAt: string | null,
  endedAt: string | null,
): string {
  if (!startedAt || !endedAt) return "—";
  const start = new Date(startedAt).getTime();
  const end = new Date(endedAt).getTime();
  if (!Number.isFinite(start) || !Number.isFinite(end) || end < start) {
    return "—";
  }

  const totalSeconds = Math.round((end - start) / 1_000);
  if (totalSeconds < 1) return "少于 1 秒";
  if (totalSeconds < 60) return `${totalSeconds} 秒`;

  const hours = Math.floor(totalSeconds / 3_600);
  const minutes = Math.floor((totalSeconds % 3_600) / 60);
  const seconds = totalSeconds % 60;
  if (hours > 0) {
    return minutes > 0 ? `${hours} 小时 ${minutes} 分钟` : `${hours} 小时`;
  }
  return seconds > 0 ? `${minutes} 分 ${seconds} 秒` : `${minutes} 分钟`;
}

export function formatDateTime(value: string | null): string {
  if (!value) return "—";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "—";
  return new Intl.DateTimeFormat(undefined, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  }).format(date);
}

export function formatScalarValue(value: unknown): string | null {
  if (value === null) return "—";
  if (typeof value === "boolean") return value ? "是" : "否";
  if (typeof value === "string" || typeof value === "number") {
    return String(value);
  }
  return null;
}

function schemaProperties(schema: unknown): Record<string, unknown> {
  if (!isRecord(schema) || !isRecord(schema.properties)) return {};
  return schema.properties;
}

function schemaText(
  schema: unknown,
  key: "description" | "title",
): string | null {
  if (!isRecord(schema)) return null;
  const value = schema[key];
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
