import type { Viewport, XYPosition } from "@xyflow/react";
import { workflowConnections } from "./workflowGraphOperations.ts";
import type { WorkflowNodeSelection } from "./workflowNodeSelection.ts";

const STORAGE_PREFIX = "opentopia.flow-canvas-layout.v1:";
const NODE_WIDTH = 260;
const COLUMN_GAP = 112;
const ROW_GAP = 72;
const CANVAS_INSET = 80;

export type WorkflowCanvasLayout = {
  positions: Record<string, XYPosition>;
  viewport?: Viewport;
};

export function automaticWorkflowPositions(
  nodes: readonly WorkflowNodeSelection[],
): Record<string, XYPosition> {
  const ranks = new Map(nodes.map((node) => [node.id, 0]));
  const edges = workflowConnections(nodes);
  for (let pass = 0; pass < nodes.length; pass += 1) {
    let changed = false;
    for (const edge of edges) {
      const nextRank = (ranks.get(edge.sourceId) ?? 0) + 1;
      if (nextRank > (ranks.get(edge.targetId) ?? 0)) {
        ranks.set(edge.targetId, nextRank);
        changed = true;
      }
    }
    if (!changed) break;
  }

  const rowsByRank = new Map<number, string[]>();
  for (const node of nodes) {
    const rank = ranks.get(node.id) ?? 0;
    rowsByRank.set(rank, [...(rowsByRank.get(rank) ?? []), node.id]);
  }

  return Object.fromEntries(
    nodes.map((node) => {
      const rank = ranks.get(node.id) ?? 0;
      const row = rowsByRank.get(rank)?.indexOf(node.id) ?? 0;
      return [
        node.id,
        {
          x: CANVAS_INSET + rank * (NODE_WIDTH + COLUMN_GAP),
          y: CANVAS_INSET + row * (156 + ROW_GAP),
        },
      ];
    }),
  );
}

export function reconcileWorkflowPositions(
  nodes: readonly WorkflowNodeSelection[],
  positions: Record<string, XYPosition>,
): Record<string, XYPosition> {
  const automatic = automaticWorkflowPositions(nodes);
  return Object.fromEntries(
    nodes.map((node) => [node.id, positions[node.id] ?? automatic[node.id]!]),
  );
}

export function readWorkflowCanvasLayout(
  layoutId: string,
  nodes: readonly WorkflowNodeSelection[],
): WorkflowCanvasLayout {
  const automatic = automaticWorkflowPositions(nodes);
  if (typeof window === "undefined") return { positions: automatic };
  try {
    const raw = window.localStorage.getItem(storageKey(layoutId));
    if (!raw) return { positions: automatic };
    const parsed = JSON.parse(raw) as Partial<WorkflowCanvasLayout>;
    const positions = isPositionRecord(parsed.positions)
      ? parsed.positions
      : automatic;
    return {
      positions: reconcileWorkflowPositions(nodes, positions),
      ...(isViewport(parsed.viewport) ? { viewport: parsed.viewport } : {}),
    };
  } catch {
    return { positions: automatic };
  }
}

export function writeWorkflowCanvasLayout(
  layoutId: string,
  layout: WorkflowCanvasLayout,
) {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(storageKey(layoutId), JSON.stringify(layout));
  } catch {
    // Layout persistence is best effort and must never block Flow editing.
  }
}

function storageKey(layoutId: string) {
  return `${STORAGE_PREFIX}${layoutId}`;
}

function isPositionRecord(value: unknown): value is Record<string, XYPosition> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  return Object.values(value).every(isPosition);
}

function isPosition(value: unknown): value is XYPosition {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const candidate = value as Partial<XYPosition>;
  return Number.isFinite(candidate.x) && Number.isFinite(candidate.y);
}

function isViewport(value: unknown): value is Viewport {
  if (!isPosition(value)) return false;
  const candidate = value as Partial<Viewport>;
  return Number.isFinite(candidate.zoom) && candidate.zoom! > 0;
}
