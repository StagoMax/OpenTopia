export const FILE_PATH_LINK_SCHEME = "opentopia-file:";
export const FILE_PATH_LINK_CLASS = "markdown-path-link";
export const FILE_PATH_LINK_TEXT_CLASS = "markdown-path-link-text";

export type DetectedFilePath = {
  /** Token as written, including any `:42` line suffix. */
  raw: string;
  /** Path text only, with separators normalised to '/'. */
  path: string;
  /** Line number from a `file.ts:42` suffix, when present. */
  line: number | null;
};

export type FilePathMatch = {
  start: number;
  end: number;
  detected: DetectedFilePath;
};

const PATH_CANDIDATE =
  /(?:[A-Za-z]:)?[\\/]?(?:[\w~+.-]+[\\/])*[\w~+.-]+(?::\d+(?::\d+)?)?/g;
const LINE_SUFFIX = /^(.*?):(\d+)(?::\d+)?$/;
const WINDOWS_DRIVE = /^[A-Za-z]:[\\/]/;

/**
 * Parses a single token such as `docs/guide.md:42` into a workspace path.
 * Bare file names (`README.md`) are only accepted when the token is already
 * known to be path-shaped — inside a code span, for example — because prose
 * words like `Node.js` are indistinguishable otherwise.
 */
export function parseFilePathToken(
  token: string,
  options: { allowBareFileName?: boolean } = {},
): DetectedFilePath | null {
  const raw = token.trim().replace(/\.+$/, "");
  if (!raw || raw.includes("://") || raw.includes("@")) return null;

  const suffix = LINE_SUFFIX.exec(raw);
  const rawPath = suffix ? suffix[1] : raw;
  const line = suffix ? Number.parseInt(suffix[2], 10) : null;
  if (!rawPath) return null;

  const normalized = rawPath.replaceAll("\\", "/");
  const drive = WINDOWS_DRIVE.test(rawPath);
  if (normalized.slice(drive ? 2 : 0).includes(":")) return null;

  const segments = normalized.split("/").filter(Boolean);
  const fileName = segments.at(-1);
  if (!fileName || !looksLikeFileName(fileName)) return null;

  const hasSeparator = drive || normalized.includes("/");
  if (!hasSeparator && !options.allowBareFileName) return null;

  return { raw, path: normalized, line };
}

/** Finds every path-shaped token inside a prose run. */
export function findFilePaths(value: string): FilePathMatch[] {
  const matches: FilePathMatch[] = [];
  for (const match of value.matchAll(PATH_CANDIDATE)) {
    const start = match.index ?? 0;
    if (isJoinedToPrecedingText(value, start)) continue;
    const detected = parseFilePathToken(match[0]);
    if (!detected) continue;
    matches.push({ start, end: start + detected.raw.length, detected });
  }
  return matches;
}

export function encodeFilePathHref(detected: DetectedFilePath): string {
  const fragment = detected.line == null ? "" : `#L${detected.line}`;
  return `${FILE_PATH_LINK_SCHEME}${encodeURIComponent(detected.path)}${fragment}`;
}

export function decodeFilePathHref(
  href: string,
): { path: string; fragment: string | null } | null {
  if (!href.startsWith(FILE_PATH_LINK_SCHEME)) return null;
  const value = href.slice(FILE_PATH_LINK_SCHEME.length);
  const hashIndex = value.indexOf("#");
  const encodedPath = hashIndex >= 0 ? value.slice(0, hashIndex) : value;
  const fragment = hashIndex >= 0 ? value.slice(hashIndex + 1) || null : null;
  try {
    return { path: decodeURIComponent(encodedPath), fragment };
  } catch {
    return null;
  }
}

export function isWindowsDrivePath(value: string): boolean {
  return WINDOWS_DRIVE.test(value);
}

type MdastNode = {
  type: string;
  value?: string;
  url?: string;
  title?: string | null;
  children?: MdastNode[];
  data?: { hProperties?: Record<string, string> };
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

/**
 * Remark plugin that turns bare file paths into links so a click can open them
 * in the preview panel. Markdown links, code blocks and images are left alone.
 */
export function remarkFilePathLinks() {
  return (tree: unknown) => {
    transformChildren(tree as MdastNode);
  };
}

function transformChildren(node: MdastNode): void {
  const children = node.children;
  if (!children || children.length === 0) return;

  const next: MdastNode[] = [];
  let changed = false;
  for (const child of children) {
    if (OPAQUE_NODES.has(child.type)) {
      next.push(child);
      continue;
    }
    if (child.type === "text") {
      const replacement = linkifyText(child.value ?? "");
      if (replacement) {
        next.push(...replacement);
        changed = true;
      } else {
        next.push(child);
      }
      continue;
    }
    if (child.type === "inlineCode") {
      const link = linkifyInlineCode(child);
      if (link) {
        next.push(link);
        changed = true;
      } else {
        next.push(child);
      }
      continue;
    }
    transformChildren(child);
    next.push(child);
  }

  if (changed) node.children = next;
}

function linkifyText(value: string): MdastNode[] | null {
  const matches = findFilePaths(value);
  if (matches.length === 0) return null;

  const nodes: MdastNode[] = [];
  let cursor = 0;
  for (const match of matches) {
    if (match.start > cursor) {
      nodes.push({ type: "text", value: value.slice(cursor, match.start) });
    }
    nodes.push(
      filePathLink(match.detected, [
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

function linkifyInlineCode(node: MdastNode): MdastNode | null {
  const detected = parseFilePathToken(node.value ?? "", {
    allowBareFileName: true,
  });
  if (!detected) return null;
  return filePathLink(detected, [node], false);
}

function filePathLink(
  detected: DetectedFilePath,
  children: MdastNode[],
  monospace = true,
): MdastNode {
  const className = monospace
    ? `${FILE_PATH_LINK_CLASS} ${FILE_PATH_LINK_TEXT_CLASS}`
    : FILE_PATH_LINK_CLASS;
  return {
    type: "link",
    url: encodeFilePathHref(detected),
    title: detected.raw,
    data: { hProperties: { className } },
    children,
  };
}

function looksLikeFileName(segment: string): boolean {
  if (/^\.{1,2}$/.test(segment)) return false;
  if (/\.[A-Za-z][\w+-]{0,9}$/.test(segment)) return true;
  return /^\.[A-Za-z][\w.+-]*$/.test(segment);
}

function isJoinedToPrecedingText(value: string, start: number): boolean {
  if (start === 0) return false;
  return /[\w~+.\-/\\]/.test(value[start - 1]);
}
