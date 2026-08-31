import assert from "node:assert/strict";
import test from "node:test";
import {
  composerTextInsertionValue,
  ensureComposerAtomicTextBoundaries,
  stabilizeComposerCaretRange,
} from "./composerDom.ts";

class FakeDocument {
  createTextNode(data: string) {
    return new FakeNode(this, 3, data);
  }
}

class FakeNode {
  readonly ownerDocument: FakeDocument;
  readonly nodeType: number;
  readonly data: string;
  parentNode: FakeNode | null = null;
  childNodes: FakeNode[] = [];
  dataset: Record<string, string> = {};
  classList = {
    contains: (name: string) => this.classes.has(name),
  };
  private readonly classes = new Set<string>();

  constructor(ownerDocument: FakeDocument, nodeType: number, data = "") {
    this.ownerDocument = ownerDocument;
    this.nodeType = nodeType;
    this.data = data;
  }

  get previousSibling(): FakeNode | null {
    if (!this.parentNode) return null;
    const index = this.parentNode.childNodes.indexOf(this);
    return index > 0 ? this.parentNode.childNodes[index - 1] : null;
  }

  get nextSibling(): FakeNode | null {
    if (!this.parentNode) return null;
    const index = this.parentNode.childNodes.indexOf(this);
    return index >= 0 ? (this.parentNode.childNodes[index + 1] ?? null) : null;
  }

  append(...nodes: FakeNode[]) {
    for (const node of nodes) {
      node.parentNode = this;
      this.childNodes.push(node);
    }
  }

  insertBefore(node: FakeNode, reference: FakeNode | null) {
    node.parentNode = this;
    if (!reference) this.childNodes.push(node);
    else this.childNodes.splice(this.childNodes.indexOf(reference), 0, node);
    return node;
  }

  contains(node: FakeNode | null): boolean {
    if (!node) return false;
    return (
      node === this || this.childNodes.some((child) => child.contains(node))
    );
  }

  addClass(name: string) {
    this.classes.add(name);
  }
}

class FakeRange {
  readonly commonAncestorContainer: FakeNode;
  startContainer: FakeNode;
  startOffset: number;
  collapsed = true;
  endContainer: FakeNode;
  endOffset: number;

  constructor(
    commonAncestorContainer: FakeNode,
    startContainer: FakeNode,
    startOffset: number,
  ) {
    this.commonAncestorContainer = commonAncestorContainer;
    this.startContainer = startContainer;
    this.startOffset = startOffset;
    this.endContainer = startContainer;
    this.endOffset = startOffset;
  }

  cloneRange() {
    return new FakeRange(
      this.commonAncestorContainer,
      this.startContainer,
      this.startOffset,
    );
  }

  setStart(node: FakeNode, offset: number) {
    this.startContainer = node;
    this.startOffset = offset;
  }

  collapse() {
    this.collapsed = true;
    this.endContainer = this.startContainer;
    this.endOffset = this.startOffset;
  }
}

function atomicNode(document: FakeDocument) {
  const node = new FakeNode(document, 1);
  node.dataset.composerAtomicReference = "true";
  node.addClass("composer-attachment-reference");
  return node;
}

test("adds a caret marker only to otherwise invisible trailing line breaks", () => {
  assert.equal(composerTextInsertionValue("\n"), "\n\u200b");
  assert.equal(composerTextInsertionValue("\n2. "), "\n2. ");
  assert.equal(composerTextInsertionValue("\n\u200b1. "), "\n1. ");
});

test("gives an atomic reference stable editable text boundaries", () => {
  const document = new FakeDocument();
  const editor = new FakeNode(document, 1);
  const atom = atomicNode(document);
  editor.append(atom);

  const boundaries = ensureComposerAtomicTextBoundaries(
    atom as unknown as HTMLElement,
  );

  assert.ok(boundaries);
  assert.deepEqual(editor.childNodes, [
    boundaries.before,
    atom,
    boundaries.after,
  ]);
  assert.equal(boundaries.before.data, "");
  assert.equal(boundaries.after.data, "");
  assert.equal(
    ensureComposerAtomicTextBoundaries(atom as unknown as HTMLElement)?.after,
    boundaries.after,
  );
});

test("adjacent atomic references share the editable boundary between them", () => {
  const document = new FakeDocument();
  const editor = new FakeNode(document, 1);
  const first = atomicNode(document);
  const second = atomicNode(document);
  editor.append(first, second);

  const firstBoundaries = ensureComposerAtomicTextBoundaries(
    first as unknown as HTMLElement,
  );
  const secondBoundaries = ensureComposerAtomicTextBoundaries(
    second as unknown as HTMLElement,
  );

  assert.ok(firstBoundaries);
  assert.ok(secondBoundaries);
  assert.equal(firstBoundaries.after, secondBoundaries.before);
  assert.deepEqual(editor.childNodes, [
    firstBoundaries.before,
    first,
    firstBoundaries.after,
    second,
    secondBoundaries.after,
  ]);
});

test("moves a parent boundary after an atomic reference into its text anchor", () => {
  const document = new FakeDocument();
  const editor = new FakeNode(document, 1);
  const atom = atomicNode(document);
  editor.append(atom);
  const boundaries = ensureComposerAtomicTextBoundaries(
    atom as unknown as HTMLElement,
  );
  assert.ok(boundaries);
  const sourceRange = new FakeRange(editor, editor, 2);

  const stableRange = stabilizeComposerCaretRange(
    editor as unknown as HTMLElement,
    sourceRange as unknown as Range,
  );

  assert.equal(stableRange.startContainer, boundaries.after);
  assert.equal(stableRange.startOffset, 0);
  assert.equal(sourceRange.startContainer, editor);
  assert.equal(sourceRange.startOffset, 2);
});
