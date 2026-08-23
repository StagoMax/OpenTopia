import type { InlineImageAttachment, InlineMessageContentPart } from "./types";
import { workspaceRootKey } from "./workspaceRootKey.ts";

export const COMPOSER_CARET_MARKER = "\u200b";

export function composerImageDisplayId(imageId: string): string {
  return imageId.slice(0, 8);
}

export function composerAttachmentReferenceText(name: string): string {
  return `[${name}]`;
}

export type ComposerHistorySnapshot = {
  parts: InlineMessageContentPart[];
  caretOffset: number;
};

export type ComposerExternalValueSyncAction = "ignore" | "defer" | "apply";

const composerOrderedListMarkerPattern = /^( {0,3})(\d+)\.[ \t]+/;

export function composerExternalValueSyncAction({
  value,
  lastLocallyPublishedValue,
  compositionPending,
  pendingLocalPublish = false,
  lastExternalValue,
}: {
  value: string;
  lastLocallyPublishedValue: string | null;
  compositionPending: boolean;
  /**
   * The composer publishes its local text after a short idle delay. While that
   * work is queued, React can re-render with the last prop value; that is not
   * an external reset and must not overwrite the browser's live edit.
   */
  pendingLocalPublish?: boolean;
  lastExternalValue?: string;
}): ComposerExternalValueSyncAction {
  if (value === lastLocallyPublishedValue) return "ignore";
  if (pendingLocalPublish && value === lastExternalValue) return "ignore";
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

export function composerLineBreakText(
  snapshot: ComposerHistorySnapshot,
  continueOrderedList: boolean,
): string {
  return continueOrderedList
    ? (composerOrderedListContinuation(snapshot) ?? "\n")
    : "\n";
}

/**
 * Builds the text inserted by a line break inside a plain-text ordered-list
 * item. Image references on earlier lines do not affect the current line, but
 * an image on the current line keeps that line from being treated as a list.
 * The composer stores caret offsets as grapheme counts, so slice through the
 * same representation instead of using UTF-16 offsets.
 */
export function composerOrderedListContinuation(
  snapshot: ComposerHistorySnapshot,
): string | null {
  const currentLine = composerLineBeforeCaret(snapshot);
  if (currentLine === null) return null;
  return orderedListContinuation(currentLine);
}

function composerLineBeforeCaret(
  snapshot: ComposerHistorySnapshot,
): string | null {
  const beforeCaret = composerContentTokens(snapshot.parts).slice(
    0,
    Math.max(0, snapshot.caretOffset),
  );
  let currentLine: string | null = "";
  for (const token of beforeCaret) {
    if (token.type !== "text") {
      currentLine = null;
      continue;
    }
    const lastLineBreak = token.text.lastIndexOf("\n");
    if (lastLineBreak >= 0) {
      currentLine = token.text.slice(lastLineBreak + 1);
    } else if (currentLine !== null) {
      currentLine += token.text;
    }
  }
  return currentLine;
}

function orderedListContinuation(currentLine: string): string | null {
  // CommonMark allows up to three leading spaces. Requiring the marker at the
  // beginning also keeps prose such as "说明 1. 内容" from becoming a list.
  const match = composerOrderedListMarkerPattern.exec(currentLine);
  if (!match) return null;

  return `\n${match[1]}${BigInt(match[2]) + 1n}. `;
}

/**
 * The composer presents a draft that starts with a CommonMark ordered-list
 * marker with the same leading indentation as a rendered list. Keep the
 * indentation visual so the submitted text remains portable Markdown.
 */
export function composerUsesOrderedListIndentation(text: string): boolean {
  return text
    .split(/\r?\n/)
    .some((line) => composerOrderedListMarkerPattern.test(line));
}

type ComposerContentToken =
  | { type: "text"; text: string }
  | { type: "image_ref"; imageId: string }
  | { type: "attachment_ref"; path: string; name: string };

const graphemeSegmenter = new Intl.Segmenter(undefined, {
  granularity: "grapheme",
});

export function splitComposerText(text: string): string[] {
  return Array.from(graphemeSegmenter.segment(text), (part) => part.segment);
}

export function composerTextLength(text: string): number {
  if (/^[\x00-\x7f]*$/.test(text)) return text.length;
  let length = 0;
  for (const _part of graphemeSegmenter.segment(text)) length += 1;
  return length;
}

function composerContentTokens(
  parts: InlineMessageContentPart[],
): ComposerContentToken[] {
  const tokens: ComposerContentToken[] = [];
  for (const part of normalizeComposerContentParts(parts)) {
    if (part.type === "image_ref" || part.type === "attachment_ref") {
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
    if (token.type === "image_ref" || token.type === "attachment_ref") {
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
      : left.type === "attachment_ref" && right.type === "attachment_ref"
        ? left.path === right.path
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

/**
 * Chromium can replace a deleted `contenteditable=false` inline node with one
 * or more line breaks. Those breaks are browser caret scaffolding, not user
 * input, and become an apparently undeletable blank line once the composer is
 * read back. Remove them only when the same mutation actually deleted an image
 * or attachment reference and inserted nothing except line breaks.
 */
export function normalizeComposerImageDeletionSnapshot(
  before: ComposerHistorySnapshot,
  after: ComposerHistorySnapshot,
): ComposerHistorySnapshot {
  // Plain-text deletion cannot produce the Chromium artifact handled below.
  // Avoid segmenting the entire draft on the overwhelmingly common path.
  if (
    !before.parts.some(
      (part) => part.type === "image_ref" || part.type === "attachment_ref",
    )
  )
    return after;

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

  const removedTokens = beforeTokens.slice(
    prefixLength,
    beforeTokens.length - suffixLength,
  );
  const insertedTokens = afterTokens.slice(
    prefixLength,
    afterTokens.length - suffixLength,
  );
  const deletedReference = removedTokens.some(
    (token) => token.type === "image_ref" || token.type === "attachment_ref",
  );
  const insertedOnlyLineBreaks =
    insertedTokens.length > 0 &&
    insertedTokens.every(
      (token) => token.type === "text" && /^\r?\n$/.test(token.text),
    );
  if (!deletedReference || !insertedOnlyLineBreaks) return after;

  return {
    parts: composerPartsFromTokens([
      ...afterTokens.slice(0, prefixLength),
      ...afterTokens.slice(afterTokens.length - suffixLength),
    ]),
    caretOffset: prefixLength,
  };
}

export function composerUndoEntries(
  before: ComposerHistorySnapshot,
  after: ComposerHistorySnapshot,
  splitInsertedContent: boolean,
): ComposerHistorySnapshot[] {
  const beforeText = composerPlainText(before.parts);
  const afterText = composerPlainText(after.parts);
  if (beforeText !== null && afterText !== null) {
    if (beforeText === afterText) return [];
    if (!splitInsertedContent) return [cloneComposerSnapshot(before)];

    let prefixLength = 0;
    while (
      prefixLength < beforeText.length &&
      prefixLength < afterText.length &&
      beforeText[prefixLength] === afterText[prefixLength]
    ) {
      prefixLength += 1;
    }

    let suffixLength = 0;
    while (
      suffixLength < beforeText.length - prefixLength &&
      suffixLength < afterText.length - prefixLength &&
      beforeText[beforeText.length - 1 - suffixLength] ===
        afterText[afterText.length - 1 - suffixLength]
    ) {
      suffixLength += 1;
    }

    const removedCount = beforeText.length - prefixLength - suffixLength;
    const insertedText = afterText.slice(
      prefixLength,
      afterText.length - suffixLength,
    );
    if (removedCount > 0 || !insertedText) {
      return [cloneComposerSnapshot(before)];
    }

    const insertedGraphemes = splitComposerText(insertedText);
    if (insertedGraphemes.length <= 1) {
      return [cloneComposerSnapshot(before)];
    }

    const prefix = afterText.slice(0, prefixLength);
    const suffix = afterText.slice(afterText.length - suffixLength);
    const prefixCaretOffset = splitComposerText(prefix).length;
    return Array.from({ length: insertedGraphemes.length }, (_, index) =>
      index === 0
        ? cloneComposerSnapshot(before)
        : {
            parts: [
              {
                type: "text" as const,
                text:
                  prefix + insertedGraphemes.slice(0, index).join("") + suffix,
              },
            ],
            caretOffset: prefixCaretOffset + index,
          },
    );
  }

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

/**
 * Keeps inline file references aligned with the source chips currently owned
 * by the composer. The visible chip text is intentionally not used as the
 * identity: duplicate filenames are valid, while paths identify the source.
 */
export function filterComposerAttachmentReferences(
  parts: InlineMessageContentPart[],
  activePaths: Iterable<string>,
): InlineMessageContentPart[] {
  const activeKeys = new Set(
    Array.from(activePaths, (path) => workspaceRootKey(path)),
  );
  return normalizeComposerContentParts(parts).filter(
    (part) =>
      part.type !== "attachment_ref" ||
      activeKeys.has(workspaceRootKey(part.path)),
  );
}

function composerPlainText(parts: InlineMessageContentPart[]): string | null {
  let text = "";
  for (const part of normalizeComposerContentParts(parts)) {
    if (part.type !== "text") return null;
    text += part.text;
  }
  return text;
}

export function normalizeComposerContentParts(
  parts: InlineMessageContentPart[],
): InlineMessageContentPart[] {
  const normalized: InlineMessageContentPart[] = [];
  for (const part of parts) {
    if (part.type === "image_ref" || part.type === "attachment_ref") {
      normalized.push(part);
      continue;
    }
    const text = part.text.replaceAll(COMPOSER_CARET_MARKER, "");
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

export function referencedAttachmentPaths(
  parts: InlineMessageContentPart[],
): Set<string> {
  return new Set(
    parts
      .filter(
        (
          part,
        ): part is Extract<
          InlineMessageContentPart,
          { type: "attachment_ref" }
        > => part.type === "attachment_ref",
      )
      .map((part) => part.path),
  );
}

export function composerContentText(
  parts: InlineMessageContentPart[],
  attachments: InlineImageAttachment[],
): string {
  const displayIds = new Map(
    attachments.map((attachment) => [
      attachment.id,
      composerImageDisplayId(attachment.id),
    ]),
  );
  return normalizeComposerContentParts(parts)
    .map((part) => {
      if (part.type === "text") return part.text;
      if (part.type === "image_ref") {
        return `[图片：${displayIds.get(part.imageId) ?? composerImageDisplayId(part.imageId)}]`;
      }
      return composerAttachmentReferenceText(part.name);
    })
    .join("");
}

export function composerVisibleText(parts: InlineMessageContentPart[]): string {
  return normalizeComposerContentParts(parts)
    .map((part) => {
      if (part.type === "text") return part.text;
      if (part.type === "attachment_ref") {
        return composerAttachmentReferenceText(part.name);
      }
      return "";
    })
    .join("");
}

/**
 * The wire accepts only text and image reference parts. Attachment references
 * are flattened to their `[文件名]` text for submission; the actual file
 * linkage still flows through the separate `sourcePaths` channel.
 */
export function composerWireContentParts(
  parts: InlineMessageContentPart[],
): InlineMessageContentPart[] {
  return normalizeComposerContentParts(parts).map((part) =>
    part.type === "attachment_ref"
      ? {
          type: "text" as const,
          text: composerAttachmentReferenceText(part.name),
        }
      : part,
  );
}
