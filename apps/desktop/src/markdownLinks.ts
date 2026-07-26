import { decodeFilePathHref, isWindowsDrivePath } from "./filePathLinks.ts";

export type MarkdownLinkTarget =
  | { kind: "anchor"; href: string }
  | { kind: "web"; url: string }
  | { kind: "email"; url: string }
  | { kind: "workspace"; path: string; fragment: string | null }
  | { kind: "blocked"; reason: string };

const explicitSchemePattern = /^[a-z][a-z0-9+.-]*:/i;

export function resolveMarkdownLink(
  href: string,
  baseWorkspacePath?: string | null,
): MarkdownLinkTarget {
  const value = href.trim();
  if (!value) return { kind: "blocked", reason: "Link target is empty." };

  if (value.startsWith("#")) {
    return { kind: "anchor", href: value };
  }

  const literal = decodeFilePathHref(value);
  if (literal) {
    return resolveLiteralFilePath(literal.path, literal.fragment);
  }

  if (isWindowsDrivePath(value)) {
    const { path, fragment } = splitRelativeReference(value);
    return resolveLiteralFilePath(path, fragment);
  }

  if (value.startsWith("//")) {
    return resolveAbsoluteUrl(`https:${value}`);
  }

  if (explicitSchemePattern.test(value)) {
    return resolveAbsoluteUrl(value);
  }

  const { path: rawPath, fragment } = splitRelativeReference(value);
  if (!rawPath) {
    return fragment
      ? { kind: "anchor", href: `#${fragment}` }
      : { kind: "blocked", reason: "Link target is empty." };
  }

  let decodedPath: string;
  try {
    decodedPath = decodeURIComponent(rawPath).replaceAll("\\", "/");
  } catch {
    return { kind: "blocked", reason: "Link path is not valid UTF-8." };
  }

  const absoluteFromWorkspaceRoot = decodedPath.startsWith("/");
  const baseSegments = absoluteFromWorkspaceRoot
    ? []
    : workspaceBaseDirectory(baseWorkspacePath);
  const resolved = resolveWorkspaceSegments(baseSegments, decodedPath);
  if (!resolved) {
    return {
      kind: "blocked",
      reason: "Link path escapes the active workspace.",
    };
  }

  return { kind: "workspace", path: resolved, fragment };
}

/**
 * Resolves a path that was written as a filesystem path rather than as a
 * markdown-relative link: `docs/guide.md`, `/srv/app/main.rs` or `D:\repo\a.md`.
 * Absolute paths stay absolute so the server can validate them against the
 * workspace root.
 */
function resolveLiteralFilePath(
  rawPath: string,
  fragment: string | null,
): MarkdownLinkTarget {
  const normalized = rawPath.trim().replaceAll("\\", "/");
  if (!normalized) return { kind: "blocked", reason: "Link target is empty." };

  const drive = /^[A-Za-z]:/.exec(normalized)?.[0] ?? null;
  const body = drive ? normalized.slice(drive.length) : normalized;
  const segments = collapseSegments(body);
  if (!segments) {
    return {
      kind: "blocked",
      reason: "Link path escapes the active workspace.",
    };
  }

  const joined = segments.join("/");
  if (drive) return { kind: "workspace", path: `${drive}/${joined}`, fragment };
  if (body.startsWith("/"))
    return { kind: "workspace", path: `/${joined}`, fragment };
  return { kind: "workspace", path: joined, fragment };
}

export function markdownStreamInterval(textLength: number): number {
  if (textLength >= 64 * 1024) return 200;
  if (textLength >= 16 * 1024) return 120;
  return 60;
}

function resolveAbsoluteUrl(value: string): MarkdownLinkTarget {
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    return { kind: "blocked", reason: "Link URL is invalid." };
  }

  if (url.protocol === "http:" || url.protocol === "https:") {
    return { kind: "web", url: url.toString() };
  }
  if (url.protocol === "mailto:") {
    return { kind: "email", url: url.toString() };
  }
  return {
    kind: "blocked",
    reason: `Link protocol '${url.protocol}' is not allowed.`,
  };
}

function splitRelativeReference(value: string): {
  path: string;
  fragment: string | null;
} {
  const hashIndex = value.indexOf("#");
  const withoutFragment = hashIndex >= 0 ? value.slice(0, hashIndex) : value;
  const fragment = hashIndex >= 0 ? value.slice(hashIndex + 1) || null : null;
  const queryIndex = withoutFragment.indexOf("?");
  return {
    path:
      queryIndex >= 0 ? withoutFragment.slice(0, queryIndex) : withoutFragment,
    fragment,
  };
}

function workspaceBaseDirectory(path?: string | null): string[] {
  if (!path) return [];
  const normalized = path.replaceAll("\\", "/").replace(/^\/+/, "");
  const segments = normalized.split("/").filter(Boolean);
  return segments.slice(0, -1);
}

function resolveWorkspaceSegments(base: string[], path: string): string | null {
  const segments = collapseSegments(path, base);
  return segments ? segments.join("/") : null;
}

function collapseSegments(path: string, base: string[] = []): string[] | null {
  const segments = [...base];
  for (const segment of path.replace(/^\/+/, "").split("/")) {
    if (!segment || segment === ".") continue;
    if (segment === "..") {
      if (segments.length === 0) return null;
      segments.pop();
      continue;
    }
    segments.push(segment);
  }
  return segments.length ? segments : null;
}
