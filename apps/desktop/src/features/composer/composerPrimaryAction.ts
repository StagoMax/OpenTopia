export type ComposerPrimaryAction = "submit" | "sending" | "cancel";

export function resolveComposerPrimaryAction({
  hasSendableContent,
  isSending,
  isRunning,
}: {
  hasSendableContent: boolean;
  isSending: boolean;
  isRunning: boolean;
}): ComposerPrimaryAction {
  if (isRunning && !hasSendableContent) return "cancel";
  return isSending ? "sending" : "submit";
}
