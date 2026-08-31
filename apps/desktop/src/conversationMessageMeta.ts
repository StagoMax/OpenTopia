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
  const chunks = parts.flatMap((part) => {
    if (part.type === "text") {
      return [{ text: part.text, inlineText: true, inlineAttachment: false }];
    }
    if (part.type === "proposed_plan") {
      return [{ text: part.text, inlineText: false, inlineAttachment: false }];
    }
    if (part.type === "error") {
      return [
        { text: part.message, inlineText: false, inlineAttachment: false },
      ];
    }
    if (part.type === "file_ref") {
      return [{ text: part.path, inlineText: false, inlineAttachment: false }];
    }
    if (part.type === "source_ref") {
      return [
        part.inline
          ? {
              text: `[${part.source.name}]`,
              inlineText: true,
              inlineAttachment: true,
            }
          : {
              text: part.source.path,
              inlineText: false,
              inlineAttachment: false,
            },
      ];
    }
    if (part.type === "skill_ref") {
      return [
        { text: part.skill.path, inlineText: false, inlineAttachment: false },
      ];
    }
    return [];
  });
  const text = chunks.reduce((result, chunk, index) => {
    if (index === 0) return chunk.text;
    const previous = chunks[index - 1]!;
    const joinsInline =
      previous.inlineText &&
      chunk.inlineText &&
      (previous.inlineAttachment || chunk.inlineAttachment);
    return `${result}${joinsInline ? "" : "\n\n"}${chunk.text}`;
  }, "");
  return text.replace(leadingBlankLines, "").replace(trailingBlankLines, "");
}
