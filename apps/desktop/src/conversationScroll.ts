export const conversationScrollBottomThreshold = 24;

export type ConversationScrollMetrics = {
  scrollHeight: number;
  clientHeight: number;
  scrollTop: number;
};

export function isConversationScrollNearEnd({
  scrollHeight,
  clientHeight,
  scrollTop,
}: ConversationScrollMetrics): boolean {
  return (
    scrollHeight - clientHeight - scrollTop <= conversationScrollBottomThreshold
  );
}
