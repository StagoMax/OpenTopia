import {
  composerTextLength,
  normalizeComposerContentParts,
  splitComposerText,
  type ComposerHistorySnapshot,
} from "../../composerContent.ts";
import type {
  InlineImageAttachment,
  InlineMessageContentPart,
} from "../../types";

const COMPOSER_IMAGE_EXTENSIONS = new Set([
  "bmp",
  "gif",
  "jpeg",
  "jpg",
  "png",
  "svg",
  "webp",
]);

export function isComposerImageFile(file: File): boolean {
  return (
    file.type.startsWith("image/") ||
    COMPOSER_IMAGE_EXTENSIONS.has(fileExtension(file.name).toLowerCase())
  );
}

export type ComposerImageAttachment = InlineImageAttachment & {
  previewUrl: string;
  fingerprint: string;
};

export async function imageFileFingerprint(file: File): Promise<{
  data: number[];
  fingerprint: string;
}> {
  const buffer = await file.arrayBuffer();
  const digest = await crypto.subtle.digest("SHA-256", buffer);
  const fingerprint = Array.from(new Uint8Array(digest), (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
  return {
    data: Array.from(new Uint8Array(buffer)),
    fingerprint: `${file.type}:${fingerprint}`,
  };
}

function pushComposerText(parts: InlineMessageContentPart[], text: string) {
  if (!text) return;
  const previous = parts.at(-1);
  if (previous?.type === "text") previous.text += text;
  else parts.push({ type: "text", text });
}

type ComposerCaretPoint = {
  node: Node;
  offset: number;
};

export function readComposerContent(
  editor: HTMLElement,
  caretPoint?: ComposerCaretPoint,
): ComposerHistorySnapshot {
  const parts: InlineMessageContentPart[] = [];
  let contentOffset = 0;
  let caretOffset: number | null = null;
  const appendText = (text: string) => {
    const normalized = text.replaceAll("\u200b", "");
    pushComposerText(parts, normalized);
    contentOffset += composerTextLength(normalized);
  };
  const captureElementOffset = (element: HTMLElement, offset: number) => {
    if (caretPoint?.node === element && caretPoint.offset === offset) {
      caretOffset = contentOffset;
    }
  };
  const visit = (node: Node) => {
    if (node.nodeType === Node.TEXT_NODE) {
      const text = node.textContent ?? "";
      if (caretPoint?.node === node) {
        caretOffset =
          contentOffset + composerTextLength(text.slice(0, caretPoint.offset));
      }
      appendText(text);
      return;
    }
    if (!(node instanceof HTMLElement)) return;
    const imageId = node.dataset.composerImageId;
    if (imageId) {
      captureElementOffset(node, 0);
      parts.push({ type: "image_ref", imageId });
      contentOffset += 1;
      captureElementOffset(node, node.childNodes.length);
      return;
    }
    if (node.tagName === "BR") {
      captureElementOffset(node, 0);
      appendText("\n");
      return;
    }
    const block = node !== editor && ["DIV", "P"].includes(node.tagName);
    if (block) {
      const previous = parts.at(-1);
      if (previous?.type === "text" && !previous.text.endsWith("\n")) {
        previous.text += "\n";
        contentOffset += 1;
      }
    }
    captureElementOffset(node, 0);
    node.childNodes.forEach((child, index) => {
      captureElementOffset(node, index);
      visit(child);
      captureElementOffset(node, index + 1);
    });
    if (block) appendText("\n");
  };
  captureElementOffset(editor, 0);
  editor.childNodes.forEach((child, index) => {
    captureElementOffset(editor, index);
    visit(child);
    captureElementOffset(editor, index + 1);
  });
  return {
    parts: normalizeComposerContentParts(parts),
    caretOffset: caretOffset ?? contentOffset,
  };
}

export function readComposerContentParts(
  editor: HTMLElement,
): InlineMessageContentPart[] {
  return readComposerContent(editor).parts;
}

export function cloneComposerHistorySnapshot(
  snapshot: ComposerHistorySnapshot,
): ComposerHistorySnapshot {
  return {
    parts: snapshot.parts.map((part) => ({ ...part })),
    caretOffset: snapshot.caretOffset,
  };
}

export function composerSnapshotAtSelection(
  editor: HTMLElement,
): ComposerHistorySnapshot {
  const selection = window.getSelection();
  const range =
    selection && selection.rangeCount > 0 ? selection.getRangeAt(0) : null;
  return rangeBelongsToEditor(editor, range)
    ? readComposerContent(editor, {
        node: range!.startContainer,
        offset: range!.startOffset,
      })
    : readComposerContent(editor);
}

export function renderComposerSnapshot(
  editor: HTMLElement,
  snapshot: ComposerHistorySnapshot,
  attachments: ComposerImageAttachment[],
): Range {
  const attachmentsById = new Map(
    attachments.map((attachment) => [attachment.id, attachment]),
  );
  editor.replaceChildren();
  for (const part of normalizeComposerContentParts(snapshot.parts)) {
    if (part.type === "text") {
      editor.append(document.createTextNode(part.text));
      continue;
    }
    const attachment = attachmentsById.get(part.imageId);
    if (attachment) {
      editor.append(createComposerImageReferenceNode(attachment));
    }
  }

  let remaining = Math.max(0, snapshot.caretOffset);
  const range = document.createRange();
  for (const node of editor.childNodes) {
    if (node.nodeType === Node.TEXT_NODE) {
      const text = node.textContent ?? "";
      const graphemes = splitComposerText(text);
      if (remaining <= graphemes.length) {
        range.setStart(node, graphemes.slice(0, remaining).join("").length);
        range.collapse(true);
        return range;
      }
      remaining -= graphemes.length;
      continue;
    }
    if (node instanceof HTMLElement && node.dataset.composerImageId) {
      if (remaining === 0) {
        range.setStartBefore(node);
        range.collapse(true);
        return range;
      }
      remaining -= 1;
      if (remaining === 0) {
        range.setStartAfter(node);
        range.collapse(true);
        return range;
      }
    }
  }
  range.selectNodeContents(editor);
  range.collapse(false);
  return range;
}

export function createComposerImageReferenceNode(
  attachment: ComposerImageAttachment,
): HTMLElement {
  const wrapper = document.createElement("span");
  wrapper.className = "composer-inline-image-reference";
  wrapper.dataset.composerImageId = attachment.id;
  wrapper.contentEditable = "false";

  const button = document.createElement("button");
  button.type = "button";
  button.className = "composer-inline-image-button";
  button.dataset.composerImageId = attachment.id;
  const shortId = attachment.id.slice(0, 8);
  button.title = `${attachment.name || "图片"}（ID: ${attachment.id}）`;
  button.setAttribute("aria-label", `预览图片 ${shortId}`);
  button.textContent = `[图片 · ${shortId}]`;
  wrapper.append(button);
  return wrapper;
}

export function composerAttachmentReferenceId(
  name: string,
  size = 0,
  contentType = "",
): string {
  let hash = 2166136261;
  for (const character of `${name}\u0000${size}\u0000${contentType}`) {
    hash ^= character.charCodeAt(0);
    hash = Math.imul(hash, 16777619);
  }
  return `att-${(hash >>> 0).toString(16).padStart(8, "0")}`;
}

export function createComposerAttachmentReferenceNode(
  id: string,
  label: string,
): HTMLElement {
  const reference = document.createElement("span");
  reference.className = "composer-attachment-reference";
  reference.dataset.composerAttachmentId = id;
  reference.contentEditable = "false";
  reference.textContent = `[附件 · ${label} · ${id}]`;
  reference.title = `附件 ID：${id}`;
  return reference;
}

export function rangeBelongsToEditor(
  editor: HTMLElement,
  range: Range | null,
): boolean {
  if (!range) return false;
  const container =
    range.commonAncestorContainer.nodeType === Node.TEXT_NODE
      ? range.commonAncestorContainer.parentNode
      : range.commonAncestorContainer;
  return Boolean(container && editor.contains(container));
}

export function composerRangesEqual(left: Range | null, right: Range): boolean {
  return Boolean(
    left &&
    left.startContainer === right.startContainer &&
    left.startOffset === right.startOffset &&
    left.endContainer === right.endContainer &&
    left.endOffset === right.endOffset,
  );
}

export function endOfComposerRange(editor: HTMLElement): Range {
  const range = document.createRange();
  range.selectNodeContents(editor);
  range.collapse(false);
  return range;
}

export function insertComposerTextAtSelection(
  editor: HTMLElement,
  text: string,
): boolean {
  const selection = window.getSelection();
  const range =
    selection && selection.rangeCount > 0 ? selection.getRangeAt(0) : null;
  if (!rangeBelongsToEditor(editor, range)) return false;

  range!.deleteContents();
  const textNode = document.createTextNode(text);
  range!.insertNode(textNode);
  range!.setStartAfter(textNode);
  range!.collapse(true);
  selection!.removeAllRanges();
  selection!.addRange(range!);
  return true;
}

export function composerRangeAtPoint(x: number, y: number): Range | null {
  const rangedDocument = document as Document & {
    caretRangeFromPoint?(x: number, y: number): Range | null;
    caretPositionFromPoint?(
      x: number,
      y: number,
    ): {
      offsetNode: Node;
      offset: number;
    } | null;
  };
  const direct = rangedDocument.caretRangeFromPoint?.(x, y);
  if (direct) return direct;
  const position = rangedDocument.caretPositionFromPoint?.(x, y);
  if (!position) return null;
  const range = document.createRange();
  range.setStart(position.offsetNode, position.offset);
  range.collapse(true);
  return range;
}

function fileExtension(name: string): string {
  const baseName = name.split(/[\\/]/).pop() ?? name;
  const dotIndex = baseName.lastIndexOf(".");
  return dotIndex > 0 ? baseName.slice(dotIndex + 1) : "";
}
