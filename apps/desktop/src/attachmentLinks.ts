export const ATTACHMENT_LINK_SCHEME = "opentopia-attachment:";

export type AttachmentLinkSource = {
  id: string;
  name: string;
};

export type AttachmentReferenceMatch = {
  start: number;
  end: number;
  source: AttachmentLinkSource;
};

type MdastNode = {
  type: string;
  value?: string;
  url?: string;
  title?: string | null;
  children?: MdastNode[];
};

const OPAQUE_NODES = new Set([
  "code",
  "definition",
  "html",
  "image",
  "imageReference",
  "link",
  "linkReference",
  "yaml",
]);

/** Encodes the opaque attachment identity without exposing its local path. */
export function encodeAttachmentLink(source: AttachmentLinkSource): string {
  return `${ATTACHMENT_LINK_SCHEME}${encodeURIComponent(source.id)}`;
}

export function decodeAttachmentLink(href: string): string | null {
  if (!href.startsWith(ATTACHMENT_LINK_SCHEME)) return null;
  const encodedId = href.slice(ATTACHMENT_LINK_SCHEME.length);
  if (!encodedId) return null;
  try {
    return decodeURIComponent(encodedId) || null;
  } catch {
    return null;
  }
}

/**
 * Resolves an inline-code attachment label. Exact names always win. A label
 * containing `...`/`…` may resolve only when it identifies one source in the
 * current turn, which keeps abbreviated model output useful without guessing.
 */
export function resolveAttachmentReference(
  value: string,
  sources: readonly AttachmentLinkSource[],
): AttachmentLinkSource | null {
  const reference = value.trim();
  if (!reference) return null;

  const exact = uniqueSourcesByName(sources).filter(
    (source) => normalize(source.name) === normalize(reference),
  );
  if (exact.length === 1) return exact[0];
  if (!/(?:\.\.\.|…)/.test(reference)) return null;

  const matches = uniqueSourcesById(sources).filter((source) =>
    matchesAbbreviatedName(reference, source.name),
  );
  return matches.length === 1 ? matches[0] : null;
}

/** Finds unambiguous, complete attachment names inside ordinary prose. */
export function findAttachmentReferences(
  value: string,
  sources: readonly AttachmentLinkSource[],
): AttachmentReferenceMatch[] {
  const candidates = uniqueSourcesByName(sources)
    .filter((source) => source.name.trim())
    .sort((left, right) => right.name.length - left.name.length);
  const matches: AttachmentReferenceMatch[] = [];

  for (const source of candidates) {
    const needle = source.name;
    let cursor = 0;
    while (cursor < value.length) {
      const start = value.indexOf(needle, cursor);
      if (start < 0) break;
      const end = start + source.name.length;
      if (!matches.some((match) => start < match.end && end > match.start)) {
        matches.push({ start, end, source });
      }
      cursor = Math.max(end, start + 1);
    }
  }

  return matches.sort((left, right) => left.start - right.start);
}

/**
 * Remark plugin that turns references to attachments from the current turn
 * into opaque attachment links. Existing links, images and code blocks remain
 * untouched. Inline code additionally supports a unique abbreviated name.
 */
export function remarkAttachmentLinks(options?: {
  sources?: readonly AttachmentLinkSource[];
}) {
  const sources = options?.sources ?? [];
  return (tree: unknown) => transformChildren(tree as MdastNode, sources);
}

function transformChildren(
  node: MdastNode,
  sources: readonly AttachmentLinkSource[],
): void {
  const children = node.children;
  if (!children?.length || sources.length === 0) return;

  const next: MdastNode[] = [];
  let changed = false;
  for (const child of children) {
    if (OPAQUE_NODES.has(child.type)) {
      next.push(child);
      continue;
    }
    if (child.type === "text") {
      const replacement = linkifyText(child.value ?? "", sources);
      if (replacement) {
        next.push(...replacement);
        changed = true;
      } else {
        next.push(child);
      }
      continue;
    }
    if (child.type === "inlineCode") {
      const source = resolveAttachmentReference(child.value ?? "", sources);
      if (source) {
        next.push(attachmentLink(source, [child]));
        changed = true;
      } else {
        next.push(child);
      }
      continue;
    }
    transformChildren(child, sources);
    next.push(child);
  }

  if (changed) node.children = next;
}

function linkifyText(
  value: string,
  sources: readonly AttachmentLinkSource[],
): MdastNode[] | null {
  const matches = findAttachmentReferences(value, sources);
  if (matches.length === 0) return null;

  const nodes: MdastNode[] = [];
  let cursor = 0;
  for (const match of matches) {
    if (match.start > cursor) {
      nodes.push({ type: "text", value: value.slice(cursor, match.start) });
    }
    nodes.push(
      attachmentLink(match.source, [
        { type: "text", value: value.slice(match.start, match.end) },
      ]),
    );
    cursor = match.end;
  }
  if (cursor < value.length) {
    nodes.push({ type: "text", value: value.slice(cursor) });
  }
  return nodes;
}

function attachmentLink(
  source: AttachmentLinkSource,
  children: MdastNode[],
): MdastNode {
  return {
    type: "link",
    url: encodeAttachmentLink(source),
    title: source.name,
    children,
  };
}

function uniqueSourcesByName(
  sources: readonly AttachmentLinkSource[],
): AttachmentLinkSource[] {
  const counts = new Map<string, number>();
  for (const source of uniqueSourcesById(sources)) {
    const name = normalize(source.name);
    counts.set(name, (counts.get(name) ?? 0) + 1);
  }
  return uniqueSourcesById(sources).filter(
    (source) => counts.get(normalize(source.name)) === 1,
  );
}

function uniqueSourcesById(
  sources: readonly AttachmentLinkSource[],
): AttachmentLinkSource[] {
  return [
    ...new Map(
      sources
        .filter((source) => source.id && source.name.trim())
        .map((source) => [source.id, source]),
    ).values(),
  ];
}

function matchesAbbreviatedName(reference: string, name: string): boolean {
  const parts = normalize(reference)
    .split(/(?:\.\.\.|…)+/)
    .map((part) => part.trim())
    .filter(Boolean);
  if (parts.length < 2 || parts.join("").length < 5) return false;

  const candidate = normalize(name);
  if (!candidate.startsWith(parts[0]) || !candidate.endsWith(parts.at(-1)!)) {
    return false;
  }

  let cursor = 0;
  for (const part of parts) {
    const index = candidate.indexOf(part, cursor);
    if (index < 0) return false;
    cursor = index + part.length;
  }
  return true;
}

function normalize(value: string): string {
  return value.normalize("NFKC").toLocaleLowerCase();
}
