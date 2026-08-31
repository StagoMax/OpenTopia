import {
  COMPOSER_CARET_MARKER,
  composerAttachmentReferenceText,
  composerImageDisplayId,
  composerTextLength,
  normalizeComposerContentParts,
  splitComposerText,
  type ComposerHistorySnapshot,
} from "../../composerContent.ts";
import type {
  InlineImageAttachment,
  InlineMessageContentPart,
} from "../../types";
import {
  endOfComposerRange,
  ensureComposerAtomicTextBoundaries,
  ensureComposerAtomicTextBoundariesIn,
  isComposerAtomicReferenceNode,
  isComposerTextNode,
  rangeBelongsToEditor,
} from "./composerSelection.ts";

export {
  composerRangesEqual,
  endOfComposerRange,
  ensureComposerAtomicTextBoundaries,
  insertComposerAtomicNodeAtRange,
  rangeBelongsToEditor,
  stabilizeComposerCaretRange,
} from "./composerSelection.ts";

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
    const normalized = text.replaceAll(COMPOSER_CARET_MARKER, "");
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
          contentOffset +
          composerTextLength(
            text
              .slice(0, caretPoint.offset)
              .replaceAll(COMPOSER_CARET_MARKER, ""),
          );
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
    const attachmentPath = node.dataset.composerAttachmentPath;
    if (attachmentPath) {
      captureElementOffset(node, 0);
      parts.push({
        type: "attachment_ref",
        path: attachmentPath,
        name: node.dataset.composerAttachmentName ?? "",
      });
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
      editor.append(
        document.createTextNode(composerTextInsertionValue(part.text)),
      );
      continue;
    }
    if (part.type === "attachment_ref") {
      editor.append(
        createComposerAttachmentReferenceNode(part.path, part.name),
      );
      continue;
    }
    const attachment = attachmentsById.get(part.imageId);
    if (attachment) {
      editor.append(createComposerImageReferenceNode(attachment));
    }
  }

  ensureComposerAtomicTextBoundariesIn(editor);

  let remaining = Math.max(0, snapshot.caretOffset);
  const range = editor.ownerDocument.createRange();
  for (const node of editor.childNodes) {
    if (isComposerTextNode(node)) {
      const text = node.textContent ?? "";
      const visibleText = text.replaceAll(COMPOSER_CARET_MARKER, "");
      const graphemes = splitComposerText(visibleText);
      if (remaining <= graphemes.length) {
        const visibleOffset = graphemes.slice(0, remaining).join("").length;
        const domOffset =
          remaining === graphemes.length && text.endsWith(COMPOSER_CARET_MARKER)
            ? text.length
            : visibleOffset;
        range.setStart(node, domOffset);
        range.collapse(true);
        return range;
      }
      remaining -= graphemes.length;
      continue;
    }
    if (isComposerAtomicReferenceNode(node)) {
      const boundaries = ensureComposerAtomicTextBoundaries(node);
      if (remaining === 0 && boundaries) {
        range.setStart(boundaries.before, boundaries.before.data.length);
        range.collapse(true);
        return range;
      }
      remaining -= 1;
      if (remaining === 0 && boundaries) {
        range.setStart(boundaries.after, 0);
        range.collapse(true);
        return range;
      }
    }
  }
  return endOfComposerRange(editor);
}

export function createComposerImageReferenceNode(
  attachment: ComposerImageAttachment,
): HTMLElement {
  const wrapper = document.createElement("span");
  wrapper.className = "composer-inline-image-reference";
  wrapper.dataset.composerAtomicReference = "true";
  wrapper.dataset.composerImageId = attachment.id;
  wrapper.contentEditable = "false";

  const button = document.createElement("button");
  button.type = "button";
  button.className = "composer-inline-image-button";
  button.dataset.composerImageId = attachment.id;
  const name = attachment.name || "图片";
  const displayId = composerImageDisplayId(attachment.id);
  button.title = `预览 ${name}（图片 ID：${attachment.id}）`;
  button.setAttribute("aria-label", `预览图片 ${displayId}`);
  button.textContent = `ID · ${displayId}`;
  wrapper.append(button);
  return wrapper;
}

export function createComposerAttachmentReferenceNode(
  path: string,
  name: string,
): HTMLElement {
  const reference = document.createElement("span");
  reference.className = "composer-attachment-reference";
  reference.dataset.composerAtomicReference = "true";
  reference.dataset.composerAttachmentPath = path;
  reference.dataset.composerAttachmentName = name;
  reference.contentEditable = "false";
  reference.textContent = composerAttachmentReferenceText(name);
  reference.title = name;
  return reference;
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
  const textNode = document.createTextNode(composerTextInsertionValue(text));
  range!.insertNode(textNode);
  range!.setStartAfter(textNode);
  range!.collapse(true);
  selection!.removeAllRanges();
  selection!.addRange(range!);
  return true;
}

export function composerTextInsertionValue(text: string): string {
  const normalized = text.replaceAll(COMPOSER_CARET_MARKER, "");
  return normalized.endsWith("\n")
    ? `${normalized}${COMPOSER_CARET_MARKER}`
    : normalized;
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
