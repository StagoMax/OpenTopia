import {
  buildToolActivitySummary,
  type ToolActivityGroup,
  type ToolActivityIconKind,
} from "../../toolActivity.ts";
import type { ToolCall, ToolResult } from "../../types";

export type ToolGroupExecutionSummary = {
  call: ToolCall;
  result?: ToolResult;
  finishedAt?: string;
};

export type ToolGroupPresentation = {
  iconKind: ToolActivityIconKind;
  label: string;
  running: boolean;
  settleUntil?: number;
};

export const toolGroupCompletionHoldMs = 500;

/**
 * A batch describes its current child while work is in flight, then switches
 * to a stable count once every child has settled. Keeping this derivation out
 * of the component makes streaming updates and restored history agree.
 */
export function buildToolGroupPresentation(
  group: ToolActivityGroup,
  executions: readonly ToolGroupExecutionSummary[],
  now = Date.now(),
): ToolGroupPresentation {
  const current = latestRunningExecution(executions);
  if (current) {
    return executionPresentation(current.call, true);
  }

  if (executions.length === 1) {
    return executionPresentation(executions[0].call, false);
  }

  const latestFinished = latestFinishedExecution(executions);
  const settleUntil = latestFinished
    ? latestFinished.finishedAt + toolGroupCompletionHoldMs
    : 0;
  if (executions.length > 1 && latestFinished && settleUntil > now) {
    return {
      ...executionPresentation(latestFinished.execution.call, false),
      settleUntil,
    };
  }

  return {
    iconKind: completedGroupIconKind(group, executions),
    label: completedGroupLabel(group, executions),
    running: false,
  };
}

function executionPresentation(call: ToolCall, running: boolean) {
  const activity = buildToolActivitySummary(call);
  return {
    iconKind: activity.iconKind ?? activity.kind,
    label: activity.detail
      ? `${activity.title} · ${activity.detail}`
      : activity.title,
    running,
  };
}

function latestRunningExecution(
  executions: readonly ToolGroupExecutionSummary[],
) {
  for (let index = executions.length - 1; index >= 0; index -= 1) {
    if (!executions[index].result) return executions[index];
  }
  return undefined;
}

function latestFinishedExecution(
  executions: readonly ToolGroupExecutionSummary[],
) {
  let latest:
    { execution: ToolGroupExecutionSummary; finishedAt: number } | undefined;
  for (const execution of executions) {
    if (!execution.result || !execution.finishedAt) continue;
    const finishedAt = Date.parse(execution.finishedAt);
    if (!Number.isFinite(finishedAt)) continue;
    if (!latest || finishedAt >= latest.finishedAt) {
      latest = { execution, finishedAt };
    }
  }
  return latest;
}

function completedGroupLabel(
  group: ToolActivityGroup,
  executions: readonly ToolGroupExecutionSummary[],
) {
  const count = executions.length;
  if (group === "explore") return `探索了 ${count} 处`;
  if (group === "shell") return `运行了 ${count} 个命令`;
  if (group === "edit") return `修改了 ${count} 次文件`;
  if (group === "browser") return `进行了 ${count} 个浏览器操作`;
  if (group === "computer") return `进行了 ${count} 个计算机操作`;
  if (group === "spreadsheet") return `进行了 ${count} 个表格操作`;
  if (group === "agent") return `进行了 ${count} 个子智能体操作`;
  if (group === "plan") return `更新了 ${count} 次执行计划`;
  if (group === "skill") return `进行了 ${count} 次 Skill 操作`;
  if (group === "attachment") {
    if (executions.every(({ call }) => call.name === "view_attachment")) {
      return `查看了 ${count} 张图片`;
    }
    if (executions.every(({ call }) => call.name === "read_attachment")) {
      return `读取了 ${count} 个附件`;
    }
    return `处理了 ${count} 个附件`;
  }
  if (group === "mcp") return `调用了 ${count} 个 MCP 工具`;
  return `调用了 ${count} 个工具`;
}

function completedGroupIconKind(
  group: ToolActivityGroup,
  executions: readonly ToolGroupExecutionSummary[],
): ToolActivityIconKind {
  if (group === "explore") return "search";
  if (
    group === "attachment" &&
    executions.every(({ call }) => call.name === "view_attachment")
  ) {
    return "image";
  }
  return group;
}
