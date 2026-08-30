import { useLayoutEffect } from "react";
import type { ConversationSessionRegistry } from "../../conversationSessionController";
import type { AgentEvent } from "../../types";
import { useConversationSession } from "../../useConversationSession";
import { useThreadRunState } from "../../useThreadActivityStore";
import { MessageList, type MessageListProps } from "./MessageList";

type LiveConversationMessageListProps = Omit<
  MessageListProps,
  | "messages"
  | "events"
  | "activeTurnId"
  | "pendingTurnFeedback"
  | "syncing"
  | "syncError"
  | "hasOlderMessages"
  | "loadingOlderMessages"
  | "olderMessagesError"
  | "onLoadOlderMessages"
  | "onRetrySync"
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
  const { controller, state } = useConversationSession(
    conversationRegistry,
    threadId,
  );
  const runState = useThreadRunState(
    conversationRegistry.activityStore,
    threadId,
  );
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
      activeTurnId={runState.activeTurnId}
      pendingTurnFeedback={runState.pendingTurnFeedback}
      syncing={state.syncing}
      syncError={state.syncError}
      hasOlderMessages={state.hasOlderMessages}
      loadingOlderMessages={state.loadingOlderMessages}
      olderMessagesError={state.olderMessagesError}
      onLoadOlderMessages={() =>
        controller?.loadOlderMessages() ?? Promise.resolve()
      }
      onRetrySync={() => controller?.retry()}
      onLoadToolResultDetail={controller?.loadToolResultDetail}
    />
  );
}
