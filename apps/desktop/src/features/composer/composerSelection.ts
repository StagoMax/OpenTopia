const COMPOSER_ATOMIC_REFERENCE_SELECTOR = [
  '[data-composer-atomic-reference="true"]',
  ".composer-inline-image-reference",
  ".composer-attachment-reference",
].join(", ");
const ELEMENT_NODE = 1;
const TEXT_NODE = 3;

type ComposerAtomicTextBoundaries = {
  before: Text;
  after: Text;
};

export function isComposerTextNode(node: Node | null): node is Text {
  return node?.nodeType === TEXT_NODE;
}

export function isComposerAtomicReferenceNode(
  node: Node | null,
): node is HTMLElement {
  if (node?.nodeType !== ELEMENT_NODE) return false;
  const element = node as HTMLElement;
  return (
    element.dataset.composerAtomicReference === "true" ||
    element.classList.contains("composer-inline-image-reference") ||
    element.classList.contains("composer-attachment-reference")
  );
}

/**
 * Keep every non-editable inline reference between editable text nodes.
 *
 * Chromium can leave a collapsed selection at the parent/child boundary next
 * to a contenteditable=false node. That boundary is not a stable IME target:
 * successive composition updates can be appended instead of replacing the
 * previous update. A real text node gives the browser one stable editing host
 * without adding visible or serialized content.
 */
export function ensureComposerAtomicTextBoundaries(
  atom: HTMLElement,
): ComposerAtomicTextBoundaries | null {
  const parent = atom.parentNode;
  if (!parent) return null;

  const previousSibling = atom.previousSibling;
  let before: Text;
  if (isComposerTextNode(previousSibling)) {
    before = previousSibling;
  } else {
    before = atom.ownerDocument.createTextNode("");
    parent.insertBefore(before, atom);
  }

  const nextSibling = atom.nextSibling;
  let after: Text;
  if (isComposerTextNode(nextSibling)) {
    after = nextSibling;
  } else {
    after = atom.ownerDocument.createTextNode("");
    parent.insertBefore(after, atom.nextSibling);
  }

  return { before, after };
}

export function ensureComposerAtomicTextBoundariesIn(editor: HTMLElement) {
  editor
    .querySelectorAll<HTMLElement>(COMPOSER_ATOMIC_REFERENCE_SELECTOR)
    .forEach(ensureComposerAtomicTextBoundaries);
}

export function rangeBelongsToEditor(
  editor: HTMLElement,
  range: Range | null,
): boolean {
  if (!range) return false;
  const container = isComposerTextNode(range.commonAncestorContainer)
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

export function insertComposerAtomicNodeAtRange(
  range: Range,
  node: HTMLElement,
): Range {
  range.deleteContents();
  range.insertNode(node);
  const boundaries = ensureComposerAtomicTextBoundaries(node);
  if (!boundaries) {
    throw new Error("Inserted composer atomic node has no editable parent");
  }
  const caretRange = node.ownerDocument.createRange();
  caretRange.setStart(boundaries.after, 0);
  caretRange.collapse(true);
  return caretRange;
}

export function stabilizeComposerCaretRange(
  editor: HTMLElement,
  sourceRange: Range,
): Range {
  const range = sourceRange.cloneRange();
  if (
    !sourceRange.collapsed ||
    !rangeBelongsToEditor(editor, sourceRange) ||
    sourceRange.startContainer.nodeType !== ELEMENT_NODE
  ) {
    return range;
  }

  const container = sourceRange.startContainer;
  const before =
    sourceRange.startOffset > 0
      ? (container.childNodes[sourceRange.startOffset - 1] ?? null)
      : null;
  const after = container.childNodes[sourceRange.startOffset] ?? null;

  if (isComposerAtomicReferenceNode(before)) {
    const boundaries = ensureComposerAtomicTextBoundaries(before);
    if (boundaries) range.setStart(boundaries.after, 0);
  } else if (isComposerAtomicReferenceNode(after)) {
    const boundaries = ensureComposerAtomicTextBoundaries(after);
    if (boundaries) {
      range.setStart(boundaries.before, boundaries.before.data.length);
    }
  }
  range.collapse(true);
  return range;
}

export function endOfComposerRange(editor: HTMLElement): Range {
  const range = editor.ownerDocument.createRange();
  const lastChild = editor.lastChild;
  if (isComposerTextNode(lastChild)) {
    range.setStart(lastChild, lastChild.data.length);
    range.collapse(true);
    return range;
  }
  if (isComposerAtomicReferenceNode(lastChild)) {
    const boundaries = ensureComposerAtomicTextBoundaries(lastChild);
    if (boundaries) {
      range.setStart(boundaries.after, 0);
      range.collapse(true);
      return range;
    }
  }
  range.selectNodeContents(editor);
  range.collapse(false);
  return range;
}
