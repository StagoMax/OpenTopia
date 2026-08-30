export type WorkspaceLayoutPreferences = {
  left?: number;
  contextRight?: number;
  toolRight?: number;
  inspectorRight?: number;
};

export type WorkspaceRightPanelKind = "context" | "tool" | "inspector";

export type WorkspaceLayout = {
  left: number;
  leftMin: number;
  leftMax: number;
  right: number;
  rightMin: number;
  rightMax: number;
};

export const workspaceLayoutStorageKey = "opentopia.workspace-layout.v1";

const workspaceThreePaneBreakpoint = 1120;
const workspaceLeftMin = 200;
const workspaceLeftMax = 420;

export function readWorkspaceLayoutPreferences(): WorkspaceLayoutPreferences {
  if (typeof window === "undefined") return {};
  try {
    const parsed = JSON.parse(
      window.localStorage.getItem(workspaceLayoutStorageKey) ?? "{}",
    ) as Record<string, unknown>;
    return {
      left: validStoredPanelSize(parsed.left),
      contextRight: validStoredPanelSize(parsed.contextRight),
      toolRight: validStoredPanelSize(parsed.toolRight),
      inspectorRight: validStoredPanelSize(
        parsed.inspectorRight ?? parsed.agentRight,
      ),
    };
  } catch {
    return {};
  }
}

function validStoredPanelSize(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) && value > 0
    ? value
    : undefined;
}

export function clampPanelSize(
  value: number,
  min: number,
  max: number,
): number {
  return Math.round(Math.min(Math.max(value, min), max));
}

function defaultWorkspaceLeftWidth(
  workspaceWidth: number,
  toolOnly: boolean,
): number {
  if (workspaceWidth <= 840) return toolOnly ? 210 : 226;
  if (workspaceWidth <= workspaceThreePaneBreakpoint)
    return toolOnly ? 210 : 252;
  return toolOnly ? 220 : 264;
}

export function resolveWorkspaceLayout(
  preferences: WorkspaceLayoutPreferences,
  workspaceWidth: number,
  rightPanelKind: WorkspaceRightPanelKind,
  toolOnly: boolean,
): WorkspaceLayout {
  const width = Math.max(workspaceWidth, 760);
  const compact = width <= workspaceThreePaneBreakpoint || toolOnly;
  const hasDockedPanel = rightPanelKind !== "context";
  const compactMainMin = hasDockedPanel ? 560 : 440;
  const centerMin = hasDockedPanel ? 360 : 480;
  const rightMin =
    rightPanelKind === "tool"
      ? 360
      : rightPanelKind === "inspector"
        ? 280
        : 240;
  const rightCap = rightPanelKind === "tool" ? 1200 : 520;
  const leftMax = Math.max(
    workspaceLeftMin,
    Math.min(
      workspaceLeftMax,
      width - (compact ? compactMainMin : centerMin + rightMin),
    ),
  );
  const left = clampPanelSize(
    preferences.left ?? defaultWorkspaceLeftWidth(width, toolOnly),
    workspaceLeftMin,
    leftMax,
  );
  const rightMax = Math.max(
    rightMin,
    Math.min(rightCap, width - left - centerMin),
  );
  const defaultRight =
    rightPanelKind === "tool"
      ? width - left - clampPanelSize(width * 0.31, centerMin, 600)
      : rightPanelKind === "inspector"
        ? 360
        : 286;
  const preferredRight =
    rightPanelKind === "tool"
      ? preferences.toolRight
      : rightPanelKind === "inspector"
        ? preferences.inspectorRight
        : preferences.contextRight;

  return {
    left,
    leftMin: workspaceLeftMin,
    leftMax,
    right: clampPanelSize(preferredRight ?? defaultRight, rightMin, rightMax),
    rightMin,
    rightMax,
  };
}
