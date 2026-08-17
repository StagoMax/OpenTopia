import test from "node:test";
import assert from "node:assert/strict";
import type { AgentListItem } from "./types";
import type * as AgentListStateModule from "./agentListState";

const { reuseUnchangedAgentList, sameAgentListSnapshot } = (await import(
  "./agentListState" + ".ts"
)) as typeof AgentListStateModule;

function item(cursor: number): AgentListItem {
  return {
    agent: {
      id: "agent-1",
      sessionId: "session-1",
      path: "/root/worker",
      taskName: "worker",
      agentType: "worker",
      runtimeSnapshotId: "snapshot-1",
      spawnPolicy: {
        allowChildSpawns: false,
        maxDepth: 1,
        maxDirectChildren: 0,
      },
      createdAt: "2026-08-17T00:00:00Z",
    },
    latestTurn: {
      id: "turn-1",
      sessionId: "session-1",
      agentThreadId: "agent-1",
      sequence: 1,
      taskMessage: "work",
      status: "running",
      invocationId: 1,
      createdAt: "2026-08-17T00:00:00Z",
    },
    availability: "running",
    activity: {
      agentThreadId: "agent-1",
      agentTurnId: "turn-1",
      turnStatus: "running",
      cursor,
      reasoningTail: "thinking",
      recentEvents: [],
      recentToolResults: [],
    },
  };
}

test("reuses an unchanged compact Agent snapshot", () => {
  const current = [item(12)];
  const next = [item(12)];

  assert.equal(sameAgentListSnapshot(current, next), true);
  assert.equal(reuseUnchangedAgentList(current, next), current);
});

test("accepts a snapshot when its activity cursor advances", () => {
  const current = [item(12)];
  const next = [item(13)];

  assert.equal(sameAgentListSnapshot(current, next), false);
  assert.equal(reuseUnchangedAgentList(current, next), next);
});
