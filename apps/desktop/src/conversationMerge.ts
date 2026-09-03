import type { AgentEvent, Message } from "./types";

export function mergeConversationMessages(
  current: Message[],
  incoming: Message[],
): Message[] {
  if (incoming.length === 0) return current;
  const byId = new Map(current.map((message) => [message.id, message]));
  for (const message of incoming) byId.set(message.id, message);
  return [...byId.values()].sort(
    (left, right) =>
      Date.parse(left.createdAt) - Date.parse(right.createdAt) ||
      left.id.localeCompare(right.id),
  );
}

export function mergeConversationEvents(
  current: AgentEvent[],
  incoming: AgentEvent[],
): AgentEvent[] {
  if (incoming.length === 0) return current;
  const currentTailSeq = current.at(-1)?.seq ?? Number.NEGATIVE_INFINITY;
  let previousSeq = currentTailSeq;
  const appendOnly = incoming.every((event) => {
    const ordered = event.seq > previousSeq;
    previousSeq = event.seq;
    return ordered;
  });
  if (appendOnly) {
    const compacted = [...current];
    for (const event of incoming) appendCompactedEvent(compacted, event);
    return compacted;
  }

  const byId = new Map(current.map((event) => [event.id, event]));
  for (const event of incoming) byId.set(event.id, event);
  const ordered = [...byId.values()].sort(
    (left, right) => left.seq - right.seq,
  );
  const compacted: AgentEvent[] = [];
  for (const event of ordered) appendCompactedEvent(compacted, event);
  return compacted;
}

function appendCompactedEvent(
  compacted: AgentEvent[],
  event: AgentEvent,
): void {
  const previous = compacted.at(-1);
  if (
    previous?.payload.type === "model_delta" &&
    event.payload.type === "model_delta" &&
    previous.threadId === event.threadId &&
    previous.turnId === event.turnId &&
    sameProviderAttempt(
      previous.payload.provider_attempt,
      event.payload.provider_attempt,
    )
  ) {
    compacted[compacted.length - 1] = {
      ...event,
      payload: {
        ...event.payload,
        text: previous.payload.text + event.payload.text,
      },
    };
    return;
  }
  compacted.push(event);
}

function sameProviderAttempt(
  left: Extract<
    AgentEvent["payload"],
    { type: "model_delta" }
  >["provider_attempt"],
  right: Extract<
    AgentEvent["payload"],
    { type: "model_delta" }
  >["provider_attempt"],
): boolean {
  if (!left || !right) return left === right;
  return (
    left.request_id === right.request_id &&
    left.round === right.round &&
    left.attempt === right.attempt
  );
}
