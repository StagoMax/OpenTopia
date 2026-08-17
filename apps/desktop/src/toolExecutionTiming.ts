import type { ToolResult } from "./types";

export function toolExecutionDurationMs(result?: ToolResult): number | null {
  const metadata =
    result?.metadata !== null &&
    typeof result?.metadata === "object" &&
    !Array.isArray(result.metadata)
      ? (result.metadata as Record<string, unknown>)
      : null;
  const value = metadata?.durationMs;
  const durationMs =
    typeof value === "number" && Number.isFinite(value)
      ? value
      : typeof value === "string" && /^\d+$/.test(value)
        ? Number(value)
        : null;
  return durationMs !== null && durationMs >= 0 ? durationMs : null;
}
