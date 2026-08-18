import { ArrowUp, Loader2, Square } from "lucide-react";

export function ComposerSendButton({
  hasSendableContent,
  isSending,
  isRunning,
  isCancelling,
  onSubmit,
  onCancel,
}: {
  hasSendableContent: boolean;
  isSending: boolean;
  isRunning: boolean;
  isCancelling: boolean;
  onSubmit(): void;
  onCancel(): void;
}) {
  const title = isRunning
    ? isCancelling
      ? "正在中断执行"
      : "中断执行"
    : isSending
      ? "正在发送消息"
      : "发送消息";
  const ariaLabel = isRunning
    ? isCancelling
      ? "正在中断智能体执行"
      : "中断智能体执行"
    : isSending
      ? "正在发送消息"
      : "发送消息";

  return (
    <button
      className={`send-button${hasSendableContent ? " has-content" : ""}${isSending ? " is-sending" : ""}${isRunning ? " is-running" : ""}`}
      type="button"
      disabled={isRunning ? isCancelling : isSending || !hasSendableContent}
      onClick={isRunning ? onCancel : onSubmit}
      title={title}
      aria-label={ariaLabel}
      aria-busy={isSending || isCancelling}
    >
      {isRunning ? (
        <Square
          className="stop-icon"
          size={15}
          fill="currentColor"
          aria-hidden="true"
        />
      ) : isSending ? (
        <Loader2 size={17} className="spin" aria-hidden="true" />
      ) : (
        <ArrowUp size={18} strokeWidth={2.25} aria-hidden="true" />
      )}
    </button>
  );
}
