import type { ConversationSessionRegistry } from "../../conversationSessionController";
import { useConversationSession } from "../../useConversationSession";
import { MessageList, type MessageListProps } from "./MessageList";

type LiveConversationMessageListProps = Omit<
  MessageListProps,
  "messages" | "events" | "activeTurnId" | "pendingTurnFeedback"
> & {
  conversationRegistry: ConversationSessionRegistry;
  onEventsCommitted?(events: AgentEvent[]): void;
};

/**
 * Keep the high-frequency event subscription at the conversation surface.
 * Tool events can now update this subtree without synchronously rendering the
 * application shell, sidebar, composer and workbench around it.
 */
export function LiveConversationMessageList({
  conversationRegistry,
  onEventsCommitted,
  threadId,
  ...props
}: LiveConversationMessageListProps) {
  const { state } = useConversationSession(conversationRegistry, threadId);
  const events = state?.events;
  useLayoutEffect(() => {
    if (events) onEventsCommitted?.(events);
  }, [events, onEventsCommitted]);
  if (!state || state.loadState.status !== "ready") return null;

  return (
    <MessageList
      {...props}
      threadId={threadId}
      messages={state.messages}
      events={state.events}
      activeTurnId={state.activeTurnId}
      pendingTurnFeedback={state.pendingTurnFeedback}
    />
  );
}
import { useLayoutEffect } from "react";
import type { AgentEvent } from "../../types";
