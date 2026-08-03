import type {
  InlineImageAttachment,
  InlineMessageContentPart,
} from "./types";

const caretMarker = "\u200b";

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
        (part): part is Extract<
          InlineMessageContentPart,
          { type: "image_ref" }
        > => part.type === "image_ref",
      )
      .map((part) => part.imageId),
  );
}

export function composerContentText(
  parts: InlineMessageContentPart[],
  attachments: InlineImageAttachment[],
): string {
  const names = new Map(
    attachments.map((attachment) => [
      attachment.id,
      attachment.name || "图片",
    ]),
  );
  return normalizeComposerContentParts(parts)
    .map((part) =>
      part.type === "text"
        ? part.text
        : `[图片：${names.get(part.imageId) ?? "图片"}]`,
    )
    .join("");
}
