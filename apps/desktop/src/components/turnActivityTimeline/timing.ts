import type { AgentEvent, ToolResult, WorkForm } from "../../types";
import { asRecord } from "../../toolActivity";
import { toolExecutionDurationMs } from "../../toolExecutionTiming";
import type { ToolExecution, WorkItemTiming } from "./model";

export function formatTurnTiming(
  events: AgentEvent[],
  isActive: boolean,
  now: number,
  mountedAt: number,
) {
  const validEvents = [...events]
    .sort((left, right) => left.seq - right.seq)
    .map((event) => ({ event, time: parseTimestamp(event.createdAt) }))
    .filter(
      (item): item is { event: AgentEvent; time: number } => item.time !== null,
    );
  const turnStarted = validEvents.find(
    ({ event }) => event.payload.type === "turn_started",
  );
  const startedAt =
    turnStarted?.time ?? validEvents[0]?.time ?? (isActive ? mountedAt : null);
  if (startedAt === null || startedAt === undefined) return "";

  const terminal = [...validEvents]
    .reverse()
    .find(({ event }) =>
      ["turn_finished", "turn_cancelled", "turn_suspended", "error"].includes(
        event.payload.type,
      ),
    );
  const finishedAt = isActive
    ? now
    : (terminal?.time ?? validEvents[validEvents.length - 1]?.time ?? null);
  if (finishedAt === null || finishedAt < startedAt) return "";
  return formatTurnElapsed(finishedAt - startedAt);
}

export function formatActivityTiming(
  startedAt?: string | null,
  finishedAt?: string | null,
  running = false,
  now = Date.now(),
  recordedDurationMs?: number | null,
) {
  if (
    !running &&
    recordedDurationMs !== null &&
    recordedDurationMs !== undefined
  ) {
    return `耗时 ${formatElapsed(recordedDurationMs)}`;
  }
  const start = parseTimestamp(startedAt);
  if (start === null) return "";
  const finish = running ? now : parseTimestamp(finishedAt);
  if (finish === null || finish < start) return "";
  return `${running ? "已运行" : "耗时"} ${formatElapsed(finish - start)}`;
}

export function formatExecutionGroupTiming(
  executions: ToolExecution[],
  running: boolean,
  now: number,
) {
  if (!running) {
    const recordedDurations = executions.map((execution) =>
      toolExecutionDurationMs(execution.result),
    );
    if (recordedDurations.every((duration) => duration !== null)) {
      return `耗时 ${formatElapsed(
        recordedDurations.reduce((total, duration) => total + duration, 0),
      )}`;
    }
  }
  const starts = executions
    .map((execution) => parseTimestamp(execution.startedAt))
    .filter((value): value is number => value !== null);
  if (starts.length === 0) return "";
  const start = Math.min(...starts);
  const finishes = executions
    .map((execution) => parseTimestamp(execution.finishedAt))
    .filter((value): value is number => value !== null);
  const finish = running
    ? now
    : finishes.length > 0
      ? Math.max(...finishes)
      : null;
  return formatParsedTiming(start, finish, running);
}

export function formatFileGroupTiming(files: Array<{ createdAt: string }>) {
  const times = files
    .map((file) => parseTimestamp(file.createdAt))
    .filter((value): value is number => value !== null);
  if (times.length === 0) return "";
  return formatParsedTiming(Math.min(...times), Math.max(...times), false);
}

export function formatWorkItemTiming(
  timing: WorkItemTiming | undefined,
  status: WorkForm["items"][number]["status"],
  isActive: boolean,
  now: number,
) {
  if (status === "pending") return "尚未开始";
  const formatted = formatActivityTiming(
    timing?.startedAt,
    timing?.finishedAt,
    status === "in_progress" && isActive,
    now,
  );
  return formatted || "时间不可用";
}

export function formatToolSandbox(
  result?: ToolResult,
): { label: string; detail: string; unsafe: boolean } | null {
  if (!result) return null;
  const metadata = asRecord(result.metadata);
  const escalation = metadata?.sandboxEscalation;
  if (escalation === "scoped") {
    return {
      label: "scoped approval",
      detail: "This approval applied only to the replayed tool call.",
      unsafe: false,
    };
  }
  if (escalation === "denied") {
    return {
      label: "sandbox kept",
      detail: "The approval did not disable the configured OS sandbox.",
      unsafe: false,
    };
  }

  const sandbox = asRecord(metadata?.sandbox);
  if (!sandbox || typeof sandbox.status !== "string") return null;
  const profile =
    typeof sandbox.permissionProfile === "string"
      ? sandbox.permissionProfile
      : "unknown profile";
  if (sandbox.status === "wrapped") {
    const backend =
      typeof sandbox.backend === "string" ? sandbox.backend : "OS sandbox";
    return {
      label: `${backend} / ${profile}`,
      detail: `Wrapped by ${backend} with permission profile ${profile}.`,
      unsafe: false,
    };
  }
  if (sandbox.status === "best_effort_passthrough") {
    const reason =
      typeof sandbox.reason === "string"
        ? sandbox.reason
        : "The OS sandbox backend was unavailable.";
    return { label: "sandbox passthrough", detail: reason, unsafe: true };
  }
  if (sandbox.status === "unrestricted") {
    return {
      label: "unrestricted",
      detail: "This process ran with danger-full-access.",
      unsafe: true,
    };
  }
  return {
    label: "sandbox disabled",
    detail: `OS sandbox wrapping was disabled for profile ${profile}.`,
    unsafe: true,
  };
}

function formatParsedTiming(
  startedAt: number,
  finishedAt: number | null,
  running: boolean,
) {
  if (finishedAt === null || finishedAt < startedAt) return "";
  return `${running ? "已运行" : "耗时"} ${formatElapsed(
    finishedAt - startedAt,
  )}`;
}

function parseTimestamp(value?: string | null) {
  if (!value) return null;
  const timestamp = Date.parse(value);
  return Number.isFinite(timestamp) ? timestamp : null;
}

function formatElapsed(duration: number) {
  const safeDuration = Math.max(0, Math.round(duration));
  if (safeDuration < 1_000) return `${safeDuration} ms`;
  if (safeDuration < 10_000) return `${(safeDuration / 1_000).toFixed(1)} 秒`;
  if (safeDuration < 60_000) return `${Math.round(safeDuration / 1_000)} 秒`;

  const totalSeconds = Math.round(safeDuration / 1_000);
  const seconds = totalSeconds % 60;
  const totalMinutes = Math.floor(totalSeconds / 60);
  const minutes = totalMinutes % 60;
  const hours = Math.floor(totalMinutes / 60);
  if (hours > 0) return `${hours} 小时 ${minutes} 分 ${seconds} 秒`;
  return `${minutes} 分 ${seconds} 秒`;
}

function formatTurnElapsed(duration: number) {
  const totalSeconds = Math.max(0, Math.round(duration / 1_000));
  const seconds = totalSeconds % 60;
  const totalMinutes = Math.floor(totalSeconds / 60);
  const minutes = totalMinutes % 60;
  const hours = Math.floor(totalMinutes / 60);
  if (hours > 0) return `${hours}h ${minutes}m ${seconds}s`;
  if (minutes > 0) return `${minutes}m ${seconds}s`;
  return `${seconds}s`;
}
