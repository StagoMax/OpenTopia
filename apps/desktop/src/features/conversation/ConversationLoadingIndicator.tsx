import { ShimmerText } from "../../components/ui";

export function ConversationLoadingIndicator({ label }: { label: string }) {
  return (
    <div
      className="conversation-loading-indicator"
      role="status"
      aria-label={label}
      aria-live="polite"
    >
      <ShimmerText
        className="conversation-loading-indicator__wordmark"
        aria-hidden="true"
      >
        OpenTopia
      </ShimmerText>
    </div>
  );
}
