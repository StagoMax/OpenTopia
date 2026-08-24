import { ArrowUp, Loader2, Square } from "lucide-react";
import { resolveComposerPrimaryAction } from "./composerPrimaryAction";

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
  const action = resolveComposerPrimaryAction({
    hasSendableContent,
    isSending,
    isRunning,
  });
  const isCancelAction = action === "cancel";
  const title = isCancelAction
    ? isCancelling
      ? "正在中断执行"
      : "中断执行"
    : action === "sending"
      ? "正在发送消息"
      : isRunning
        ? "追加消息"
        : "发送消息";
  const ariaLabel = isCancelAction
    ? isCancelling
      ? "正在中断智能体执行"
      : "中断智能体执行"
    : action === "sending"
      ? "正在发送消息"
      : isRunning
        ? "向正在执行的任务追加消息"
        : "发送消息";

  return (
    <button
      className={`send-button${hasSendableContent ? " has-content" : ""}${action === "sending" ? " is-sending" : ""}${isCancelAction ? " is-running" : ""}`}
      type="button"
      disabled={
        isCancelAction ? isCancelling : isSending || !hasSendableContent
      }
      onClick={isCancelAction ? onCancel : onSubmit}
      title={title}
      aria-label={ariaLabel}
      aria-busy={isSending || isCancelling}
    >
      {isCancelAction ? (
        <Square
          className="stop-icon"
          size={15}
          fill="currentColor"
          aria-hidden="true"
        />
      ) : action === "sending" ? (
        <Loader2 size={17} className="spin" aria-hidden="true" />
      ) : (
        <ArrowUp size={18} strokeWidth={2.25} aria-hidden="true" />
      )}
    </button>
  );
}
