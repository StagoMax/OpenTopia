import type { Node, NodeChange, XYPosition } from "@xyflow/react";

export function applyWorkflowNodePositions(
  positions: Readonly<Record<string, XYPosition>>,
  changes: readonly NodeChange[],
): Record<string, XYPosition> {
  let next = positions as Record<string, XYPosition>;

  for (const change of changes) {
    if (change.type !== "position" || !change.position) continue;
    const current = next[change.id];
    if (current?.x === change.position.x && current?.y === change.position.y) {
      continue;
    }
    if (next === positions) next = { ...positions };
    next[change.id] = change.position;
  }

  return next;
}

export function committedWorkflowNodePositionChanges<NodeType extends Node>(
  changes: readonly NodeChange<NodeType>[],
): NodeChange<NodeType>[] {
  return changes.filter(
    (change) => change.type === "position" && !change.dragging,
  );
}

export function reconcileWorkflowCanvasNodes<NodeType extends Node>(
  currentNodes: readonly Node[],
  projectedNodes: readonly NodeType[],
  syncPositions: boolean,
): NodeType[] {
  const currentById = new Map(currentNodes.map((node) => [node.id, node]));

  return projectedNodes.map((projectedNode) => {
    const currentNode = currentById.get(projectedNode.id);
    if (!currentNode) return projectedNode;

    return {
      ...currentNode,
      ...projectedNode,
      // React Flow owns the high-frequency gesture state. Semantic updates
      // (selection, tool, node data) must never reset an in-progress drag.
      position:
        syncPositions && !currentNode.dragging
          ? projectedNode.position
          : currentNode.position,
    } as NodeType;
  });
}
