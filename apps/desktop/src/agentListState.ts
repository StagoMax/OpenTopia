import type { AgentListItem } from "./types";

/**
 * Agent activity is refreshed from a compact snapshot. Keep the existing
 * object graph when the snapshot did not advance so unrelated desktop UI does
 * not render again for coalesced activity notifications.
 */
export function reuseUnchangedAgentList(
  current: AgentListItem[],
  next: AgentListItem[],
): AgentListItem[] {
  return sameAgentListSnapshot(current, next) ? current : next;
}

export function sameAgentListSnapshot(
  left: AgentListItem[],
  right: AgentListItem[],
): boolean {
  if (left === right) return true;
  if (left.length !== right.length) return false;
  return left.every((item, index) => sameAgentListItem(item, right[index]));
}

function sameAgentListItem(
  left: AgentListItem,
  right: AgentListItem | undefined,
): boolean {
  if (!right) return false;
  const leftAgent = left.agent;
  const rightAgent = right.agent;
  const leftTurn = left.latestTurn;
  const rightTurn = right.latestTurn;
  const leftActivity = left.activity;
  const rightActivity = right.activity;
  return (
    left.availability === right.availability &&
    leftAgent.id === rightAgent.id &&
    leftAgent.sessionId === rightAgent.sessionId &&
    leftAgent.parentAgentThreadId === rightAgent.parentAgentThreadId &&
    leftAgent.path === rightAgent.path &&
    leftAgent.taskName === rightAgent.taskName &&
    leftAgent.agentType === rightAgent.agentType &&
    leftAgent.runtimeSnapshotId === rightAgent.runtimeSnapshotId &&
    leftAgent.archivedAt === rightAgent.archivedAt &&
    leftAgent.spawnPolicy.allowChildSpawns ===
      rightAgent.spawnPolicy.allowChildSpawns &&
    leftAgent.spawnPolicy.maxDepth === rightAgent.spawnPolicy.maxDepth &&
    leftAgent.spawnPolicy.maxDirectChildren ===
      rightAgent.spawnPolicy.maxDirectChildren &&
    leftTurn?.id === rightTurn?.id &&
    leftTurn?.status === rightTurn?.status &&
    leftTurn?.invocationId === rightTurn?.invocationId &&
    leftTurn?.outcomeRef === rightTurn?.outcomeRef &&
    leftTurn?.startedAt === rightTurn?.startedAt &&
    leftTurn?.completedAt === rightTurn?.completedAt &&
    leftActivity?.agentTurnId === rightActivity?.agentTurnId &&
    leftActivity?.turnStatus === rightActivity?.turnStatus &&
    leftActivity?.cursor === rightActivity?.cursor
  );
}
