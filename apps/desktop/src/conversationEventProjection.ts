import { isContextCompactionActivityEvent } from "./conversationEventKinds.ts";
import { isSuccessfulWorkspaceWriteEvent } from "./turnChangeOwnership.ts";
import type { AgentEvent, TurnChangeSet } from "./types";

export type ConversationEventProjection = {
  eventsByTurn: Map<string, AgentEvent[]>;
  turnIdsByUserMessage: Map<string, string[]>;
  turnIdsByAssistantMessage: Map<string, string[]>;
  changeSetsByTurn: Map<string, TurnChangeSet>;
  revertedTurnIds: Set<string>;
  orphanContextActivityTurnIds: string[];
  orphanTurnErrors: AgentEvent[];
  turnsWithAssistantCards: Set<string>;
  settledTurnIds: Set<string>;
};

export function projectConversationEvents(
  events: readonly AgentEvent[],
  previous?: ConversationEventProjection,
): ConversationEventProjection {
  const eventsByTurn = new Map<string, AgentEvent[]>();
  const turnIdsByUserMessage = new Map<string, string[]>();
  const turnIdsByAssistantMessage = new Map<string, string[]>();
  const recordedChangeSets = new Map<string, TurnChangeSet>();
  const terminalTurnIds = new Set<string>();
  const successfulWriteTurnIds = new Set<string>();
  const cancelledOrFailedTurnIds = new Set<string>();
  const revertedTurnIds = new Set<string>();
  const contextActivityTurnIds = new Set<string>();
  const turnErrors: AgentEvent[] = [];

  for (const event of events) {
    const turnId = event.turnId;
    if (turnId) {
      const turnEvents = eventsByTurn.get(turnId) ?? [];
      turnEvents.push(event);
      eventsByTurn.set(turnId, turnEvents);
    }
    if (turnId && event.payload.type === "turn_started") {
      appendUnique(turnIdsByUserMessage, event.payload.user_message_id, turnId);
    }
    if (turnId && event.payload.type === "assistant_message") {
      appendUnique(turnIdsByAssistantMessage, event.payload.message.id, turnId);
    }
    if (turnId && event.payload.type === "turn_changes_recorded") {
      recordedChangeSets.set(turnId, event.payload.change_set);
      if (event.payload.change_set.revertedAt) revertedTurnIds.add(turnId);
    }
    if (event.payload.type === "turn_undo_completed") {
      revertedTurnIds.add(event.payload.target_turn_id);
    }
    if (
      turnId &&
      ["turn_finished", "turn_cancelled", "error"].includes(event.payload.type)
    ) {
      terminalTurnIds.add(turnId);
    }
    if (turnId && ["turn_cancelled", "error"].includes(event.payload.type)) {
      cancelledOrFailedTurnIds.add(turnId);
    }
    if (turnId && isSuccessfulWorkspaceWriteEvent(event)) {
      successfulWriteTurnIds.add(turnId);
    }
    if (turnId && isContextCompactionActivityEvent(event)) {
      contextActivityTurnIds.add(turnId);
    }
    if (event.payload.type === "error") turnErrors.push(event);
  }

  const changeSetsByTurn = new Map<string, TurnChangeSet>();
  for (const [turnId, changeSet] of recordedChangeSets) {
    if (
      terminalTurnIds.has(turnId) &&
      (successfulWriteTurnIds.has(turnId) ||
        !cancelledOrFailedTurnIds.has(turnId))
    ) {
      changeSetsByTurn.set(turnId, changeSet);
    }
  }

  const anchoredTurnIds = new Set(
    [...turnIdsByUserMessage.values()].flatMap((turnIds) => turnIds),
  );
  const orphanTurnErrors = reuseArray(
    previous?.orphanTurnErrors,
    turnErrors.filter(
      (event) => !event.turnId || !anchoredTurnIds.has(event.turnId),
    ),
  );
  const orphanContextActivityTurnIds = reuseArray(
    previous?.orphanContextActivityTurnIds,
    [...contextActivityTurnIds]
      .filter((turnId) => !anchoredTurnIds.has(turnId))
      .sort(
        (left, right) =>
          (eventsByTurn.get(left)?.[0]?.seq ?? 0) -
          (eventsByTurn.get(right)?.[0]?.seq ?? 0),
      ),
  );

  return {
    eventsByTurn: stabilizeArrayMap(previous?.eventsByTurn, eventsByTurn),
    turnIdsByUserMessage: stabilizeArrayMap(
      previous?.turnIdsByUserMessage,
      turnIdsByUserMessage,
    ),
    turnIdsByAssistantMessage: stabilizeArrayMap(
      previous?.turnIdsByAssistantMessage,
      turnIdsByAssistantMessage,
    ),
    changeSetsByTurn,
    revertedTurnIds,
    orphanContextActivityTurnIds,
    orphanTurnErrors,
    turnsWithAssistantCards: new Set(
      [...turnIdsByAssistantMessage.values()].flatMap((turnIds) => turnIds),
    ),
    settledTurnIds: terminalTurnIds,
  };
}

function appendUnique(
  target: Map<string, string[]>,
  key: string,
  value: string,
): void {
  const values = target.get(key) ?? [];
  if (!values.includes(value)) values.push(value);
  target.set(key, values);
}

function stabilizeArrayMap<T>(
  previous: ReadonlyMap<string, T[]> | undefined,
  next: Map<string, T[]>,
): Map<string, T[]> {
  for (const [key, values] of next) {
    const existing = previous?.get(key);
    if (existing && sameArray(existing, values)) next.set(key, existing);
  }
  return next;
}

function reuseArray<T>(previous: T[] | undefined, next: T[]): T[] {
  return previous && sameArray(previous, next) ? previous : next;
}

function sameArray<T>(left: readonly T[], right: readonly T[]): boolean {
  return (
    left.length === right.length &&
    left.every((value, index) => value === right[index])
  );
}
