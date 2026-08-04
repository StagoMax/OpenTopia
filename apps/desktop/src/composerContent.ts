import type { InlineImageAttachment, InlineMessageContentPart } from "./types";

const caretMarker = "\u200b";

export type ComposerHistorySnapshot = {
  parts: InlineMessageContentPart[];
  caretOffset: number;
};

export type ComposerExternalValueSyncAction = "ignore" | "defer" | "apply";

export function composerExternalValueSyncAction({
  value,
  lastLocallyPublishedValue,
  compositionPending,
}: {
  value: string;
  lastLocallyPublishedValue: string | null;
  compositionPending: boolean;
}): ComposerExternalValueSyncAction {
  if (value === lastLocallyPublishedValue) return "ignore";
  return compositionPending ? "defer" : "apply";
}

export function composerInputCommitPending({
  isComposing,
  compositionSnapshotPending,
  nativeIsComposing,
}: {
  isComposing: boolean;
  compositionSnapshotPending: boolean;
  nativeIsComposing: boolean;
}): boolean {
  return isComposing || compositionSnapshotPending || nativeIsComposing;
}

type ComposerContentToken =
  { type: "text"; text: string } | { type: "image_ref"; imageId: string };

const graphemeSegmenter = new Intl.Segmenter(undefined, {
  granularity: "grapheme",
});

export function splitComposerText(text: string): string[] {
  return Array.from(graphemeSegmenter.segment(text), (part) => part.segment);
}

function composerContentTokens(
  parts: InlineMessageContentPart[],
): ComposerContentToken[] {
  const tokens: ComposerContentToken[] = [];
  for (const part of normalizeComposerContentParts(parts)) {
    if (part.type === "image_ref") {
      tokens.push(part);
      continue;
    }
    tokens.push(
      ...splitComposerText(part.text).map((text) => ({
        type: "text" as const,
        text,
      })),
    );
  }
  return tokens;
}

function composerPartsFromTokens(
  tokens: ComposerContentToken[],
): InlineMessageContentPart[] {
  const parts: InlineMessageContentPart[] = [];
  for (const token of tokens) {
    if (token.type === "image_ref") {
      parts.push(token);
      continue;
    }
    const previous = parts.at(-1);
    if (previous?.type === "text") previous.text += token.text;
    else parts.push({ type: "text", text: token.text });
  }
  return parts;
}

function composerTokensEqual(
  left: ComposerContentToken,
  right: ComposerContentToken,
): boolean {
  return left.type === "text" && right.type === "text"
    ? left.text === right.text
    : left.type === "image_ref" && right.type === "image_ref"
      ? left.imageId === right.imageId
      : false;
}

function cloneComposerSnapshot(
  snapshot: ComposerHistorySnapshot,
): ComposerHistorySnapshot {
  return {
    parts: snapshot.parts.map((part) => ({ ...part })),
    caretOffset: snapshot.caretOffset,
  };
}

export function composerUndoEntries(
  before: ComposerHistorySnapshot,
  after: ComposerHistorySnapshot,
  splitInsertedContent: boolean,
): ComposerHistorySnapshot[] {
  const beforeTokens = composerContentTokens(before.parts);
  const afterTokens = composerContentTokens(after.parts);
  let prefixLength = 0;
  while (
    prefixLength < beforeTokens.length &&
    prefixLength < afterTokens.length &&
    composerTokensEqual(beforeTokens[prefixLength], afterTokens[prefixLength])
  ) {
    prefixLength += 1;
  }

  let suffixLength = 0;
  while (
    suffixLength < beforeTokens.length - prefixLength &&
    suffixLength < afterTokens.length - prefixLength &&
    composerTokensEqual(
      beforeTokens[beforeTokens.length - 1 - suffixLength],
      afterTokens[afterTokens.length - 1 - suffixLength],
    )
  ) {
    suffixLength += 1;
  }

  const removedCount = beforeTokens.length - prefixLength - suffixLength;
  const insertedTokens = afterTokens.slice(
    prefixLength,
    afterTokens.length - suffixLength,
  );
  if (removedCount === 0 && insertedTokens.length === 0) return [];
  if (!splitInsertedContent || removedCount > 0 || insertedTokens.length <= 1) {
    return [cloneComposerSnapshot(before)];
  }

  const prefix = afterTokens.slice(0, prefixLength);
  const suffix = afterTokens.slice(afterTokens.length - suffixLength);
  return Array.from({ length: insertedTokens.length }, (_, index) =>
    index === 0
      ? cloneComposerSnapshot(before)
      : {
          parts: composerPartsFromTokens([
            ...prefix,
            ...insertedTokens.slice(0, index),
            ...suffix,
          ]),
          caretOffset: prefixLength + index,
        },
  );
}

export function normalizeComposerContentParts(
  parts: InlineMessageContentPart[],
): InlineMessageContentPart[] {
  const normalized: InlineMessageContentPart[] = [];
  for (const part of parts) {
    if (part.type === "image_ref") {
      normalized.push(part);
      continue;
    }
    const text = part.text.replaceAll(caretMarker, "");
    if (!text) continue;
    const previous = normalized.at(-1);
    if (previous?.type === "text") previous.text += text;
    else normalized.push({ type: "text", text });
  }
  return normalized;
}

export function referencedImageIds(
  parts: InlineMessageContentPart[],
): Set<string> {
  return new Set(
    parts
      .filter(
        (
          part,
        ): part is Extract<InlineMessageContentPart, { type: "image_ref" }> =>
          part.type === "image_ref",
      )
      .map((part) => part.imageId),
  );
}

export function composerContentText(
  parts: InlineMessageContentPart[],
  attachments: InlineImageAttachment[],
): string {
  const names = new Map(
    attachments.map((attachment) => [attachment.id, attachment.name || "图片"]),
  );
  return normalizeComposerContentParts(parts)
    .map((part) =>
      part.type === "text"
        ? part.text
        : `[图片：${names.get(part.imageId) ?? "图片"}]`,
    )
    .join("");
}

export function composerVisibleText(parts: InlineMessageContentPart[]): string {
  return normalizeComposerContentParts(parts)
    .filter(
      (part): part is Extract<InlineMessageContentPart, { type: "text" }> =>
        part.type === "text",
    )
    .map((part) => part.text)
    .join("");
}
