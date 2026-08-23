import { shouldSubmitOnKey, type SendShortcut } from "./editorPreferences.ts";

export type ComposerEnterCommand =
  | "submit"
  | "insert-line-break"
  | "insert-list-line-break";

type ComposerEnterKey = Pick<
  KeyboardEvent,
  "altKey" | "ctrlKey" | "key" | "metaKey" | "shiftKey"
>;

/**
 * Resolves Enter once, before either the editor or an inline attachment gets
 * to interpret it. Shift+Enter is always the rich-input line-break command;
 * the send preference only selects the unshifted submit chord.
 */
export function composerEnterCommand(
  event: ComposerEnterKey,
  shortcut: SendShortcut,
): ComposerEnterCommand | null {
  if (event.key !== "Enter" || event.altKey) return null;
  if (event.shiftKey && !event.ctrlKey && !event.metaKey) {
    return "insert-list-line-break";
  }
  if (shouldSubmitOnKey(event, shortcut)) return "submit";
  if (shortcut === "mod-enter" && !event.ctrlKey && !event.metaKey) {
    return "insert-line-break";
  }
  return null;
}
