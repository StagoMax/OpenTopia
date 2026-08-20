import type { AgentEvent } from "./types";

const directWorkspaceWriteTools = new Set([
  "apply_patch",
  "create_skill",
  "spreadsheet",
  "write_file",
]);

const terminalTurnEventTypes = new Set([
  "turn_finished",
  "turn_cancelled",
  "error",
]);

function record(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

export function turnHasSuccessfulWorkspaceWrite(
  events: AgentEvent[],
  turnId: string,
): boolean {
  return events.some((event) => {
    if (
      event.turnId !== turnId ||
      event.payload.type !== "tool_call_finished"
    ) {
      return false;
    }
    const metadata = record(event.payload.result.metadata);
    if (!metadata || metadata.success === false) return false;
    if (typeof metadata.changedPath === "string") return true;
    if (
      Array.isArray(metadata.changedPaths) &&
      metadata.changedPaths.some((path) => typeof path === "string")
    ) {
      return true;
    }
    if (
      metadata.toolName === "filesystem" &&
      typeof metadata.operation === "string" &&
      ["write", "copy", "move", "delete"].includes(metadata.operation)
    ) {
      return true;
    }
    return (
      typeof metadata.toolName === "string" &&
      directWorkspaceWriteTools.has(metadata.toolName)
    );
  });
}

export function shouldShowRecordedTurnChanges(
  events: AgentEvent[],
  turnId: string,
): boolean {
  if (!turnHasTerminalEvent(events, turnId)) return false;
  if (turnHasSuccessfulWorkspaceWrite(events, turnId)) return true;
  return !events.some(
    (event) =>
      event.turnId === turnId &&
      ["turn_cancelled", "error"].includes(event.payload.type),
  );
}

export function isTurnChangeDisplaySettled(
  events: AgentEvent[],
  turnId: string,
  activeTurnId: string | null,
): boolean {
  if (activeTurnId === turnId) return false;
  return turnHasTerminalEvent(events, turnId);
}

function turnHasTerminalEvent(events: AgentEvent[], turnId: string): boolean {
  return events.some(
    (event) =>
      event.turnId === turnId &&
      terminalTurnEventTypes.has(event.payload.type),
  );
}
