import type { Message, MessagePart } from "./types";

type SourcePart = Extract<MessagePart, { type: "source_ref" }>;

/**
 * Restores attachment positions for messages written before source parts
 * carried explicit placement metadata. The migration is deliberately narrow:
 * both the source name and its bracketed text marker must be unique.
 */
export function conversationDisplayParts(message: Message): MessagePart[] {
  if (message.role !== "user") return message.parts;

  const legacySourcesByName = new Map<string, SourcePart[]>();
  for (const part of message.parts) {
    if (part.type !== "source_ref" || part.inline !== undefined) continue;
    const matches = legacySourcesByName.get(part.source.name) ?? [];
    matches.push(part);
    legacySourcesByName.set(part.source.name, matches);
  }

  const textParts = message.parts.flatMap((part) =>
    part.type === "text" ? [part.text] : [],
  );
  const sourcesByMarker = new Map<string, SourcePart>();
  for (const [name, sources] of legacySourcesByName) {
    if (sources.length !== 1) continue;
    const marker = `[${name}]`;
    const occurrences = textParts.reduce(
      (count, text) => count + literalOccurrenceCount(text, marker),
      0,
    );
    if (occurrences === 1) {
      sourcesByMarker.set(marker, sources[0]!);
    }
  }
  if (sourcesByMarker.size === 0) return message.parts;

  const restoredSourceIds = new Set(
    [...sourcesByMarker.values()].map((part) => part.source.id),
  );
  return message.parts.flatMap((part) => {
    if (part.type === "source_ref" && restoredSourceIds.has(part.source.id)) {
      return [];
    }
    if (part.type !== "text") return [part];
    return restoreTextPartSources(part.text, sourcesByMarker);
  });
}

function restoreTextPartSources(
  text: string,
  sourcesByMarker: ReadonlyMap<string, SourcePart>,
): MessagePart[] {
  const parts: MessagePart[] = [];
  let offset = 0;
  while (offset < text.length) {
    const next = [...sourcesByMarker].reduce<
      { marker: string; source: SourcePart; index: number } | undefined
    >((closest, [marker, source]) => {
      const index = text.indexOf(marker, offset);
      if (index < 0) return closest;
      if (!closest || index < closest.index) return { marker, source, index };
      if (index === closest.index && marker.length > closest.marker.length) {
        return { marker, source, index };
      }
      return closest;
    }, undefined);
    if (!next) break;
    if (next.index > offset) {
      parts.push({ type: "text", text: text.slice(offset, next.index) });
    }
    parts.push({ ...next.source, inline: true });
    offset = next.index + next.marker.length;
  }
  if (offset < text.length) {
    parts.push({ type: "text", text: text.slice(offset) });
  }
  return parts;
}

function literalOccurrenceCount(text: string, value: string): number {
  let count = 0;
  let offset = 0;
  while (offset < text.length) {
    const index = text.indexOf(value, offset);
    if (index < 0) break;
    count += 1;
    offset = index + value.length;
  }
  return count;
}
