import type { MessagePart } from "./types";

const leadingBlankLines = /^(?:[ \t\r]*\n)+/;
const trailingBlankLines = /(?:[ \t\r]*\n)*[ \t\r]*$/;

const messageWeekdayFormatter = new Intl.DateTimeFormat("zh-CN", {
  weekday: "long",
});

const messageTimeFormatter = new Intl.DateTimeFormat("zh-CN", {
  hour: "2-digit",
  minute: "2-digit",
  hour12: false,
});

const messageFullTimestampFormatter = new Intl.DateTimeFormat("zh-CN", {
  year: "numeric",
  month: "long",
  day: "numeric",
  weekday: "long",
  hour: "2-digit",
  minute: "2-digit",
  second: "2-digit",
  hour12: false,
});

export type ConversationMessageTimestamp = {
  label: string;
  title: string;
};

export function formatConversationMessageTimestamp(
  value: string,
): ConversationMessageTimestamp | null {
  const date = new Date(value);
  if (!Number.isFinite(date.getTime())) return null;
  return {
    label: `${messageWeekdayFormatter.format(date)} · ${messageTimeFormatter.format(date)}`,
    title: messageFullTimestampFormatter.format(date),
  };
}

export function conversationMessageCopyText(parts: MessagePart[]): string {
  const text = parts
    .flatMap((part) => {
      if (part.type === "text") return [part.text];
      if (part.type === "proposed_plan") return [part.text];
      if (part.type === "error") return [part.message];
      if (part.type === "file_ref") return [part.path];
      if (part.type === "source_ref") return [part.source.path];
      if (part.type === "skill_ref") return [part.skill.path];
      return [];
    })
    .join("\n\n");
  return text.replace(leadingBlankLines, "").replace(trailingBlankLines, "");
}
