import { decodeFilePathHref, isWindowsDrivePath } from "./filePathLinks.ts";

export type MarkdownLinkTarget =
  | { kind: "anchor"; href: string }
  | { kind: "web"; url: string }
  | { kind: "email"; url: string }
  | { kind: "workspace"; path: string; fragment: string | null }
  | { kind: "local"; path: string; fragment: string | null }
  | { kind: "blocked"; reason: string };

export type MarkdownFileLinkTarget = {
  kind: "workspace" | "local";
  path: string;
  fragment: string | null;
  fileName: string;
};

export type MarkdownFileLinkDisplayState = "fallback" | "label" | "link";

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
    let decodedPath: string;
    try {
      decodedPath = decodeURIComponent(path);
    } catch {
      return { kind: "blocked", reason: "Link path is not valid UTF-8." };
    }
    return resolveLiteralFilePath(decodedPath, fragment);
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

  const localBase = localBasePath(baseWorkspacePath);
  const absoluteFromWorkspaceRoot = decodedPath.startsWith("/");
  const baseSegments = absoluteFromWorkspaceRoot
    ? localBase?.drive
      ? [localBase.drive]
      : []
    : workspaceBaseDirectory(baseWorkspacePath);
  const resolved = resolveWorkspaceSegments(baseSegments, decodedPath);
  if (!resolved) {
    return {
      kind: "blocked",
      reason: "Link path escapes the active workspace.",
    };
  }

  if (localBase) {
    return {
      kind: "local",
      path: localBase.unixRoot ? `/${resolved}` : resolved,
      fragment,
    };
  }
  return { kind: "workspace", path: resolved, fragment };
}

/** Returns the display information shared by detected and explicit file links. */
export function resolveMarkdownFileLink(
  href: string,
  baseWorkspacePath?: string | null,
): MarkdownFileLinkTarget | null {
  const target = resolveMarkdownLink(href, baseWorkspacePath);
  if (target.kind !== "workspace" && target.kind !== "local") return null;

  const segments = target.path.replaceAll("\\", "/").split("/").filter(Boolean);
  return {
    kind: target.kind,
    path: target.path,
    fragment: target.fragment,
    fileName: segments.at(-1) ?? target.path,
  };
}

/**
 * Returns the path that desktop-only file actions may use. Workspace paths use
 * their resolved on-disk location; explicit absolute paths remain actionable
 * even when they cannot be previewed through the workspace-scoped API.
 */
export function markdownFileActionPath(
  target: MarkdownFileLinkTarget | null,
  workspaceAbsolutePath: string | null,
): string | null {
  if (!target) return null;
  if (workspaceAbsolutePath) return workspaceAbsolutePath;
  return /^(?:[A-Za-z]:\/|\/)/.test(target.path) ? target.path : null;
}

/**
 * Keeps an automatically detected path's filename stable while its workspace
 * lookup is pending. Explicit Markdown links keep their authored children
 * until they are known to point at a workspace file.
 */
export function markdownFileLinkDisplayState(
  automaticallyDetected: boolean,
  pathStatus: "known" | "missing" | "unknown",
): MarkdownFileLinkDisplayState {
  if (pathStatus === "known") return "link";
  return automaticallyDetected ? "label" : "fallback";
}

/**
 * Identifies explicit file links whose authored label is another spelling of
 * the target path. Those labels can be shortened synchronously without
 * replacing meaningful prose such as `[setup guide](docs/setup.md)`.
 */
export function markdownFileLinkLabelIsPath(
  label: string | null,
  target: MarkdownFileLinkTarget | null,
): boolean {
  if (!label || !target) return false;

  const { path } = splitRelativeReference(label.trim());
  let decodedPath = path;
  try {
    decodedPath = decodeURIComponent(path);
  } catch {
    // A literal percent sign is valid in a displayed filename even though it
    // is not a complete URI escape. Compare that authored spelling as-is.
  }

  const normalizedLabel = decodedPath
    .replaceAll("\\", "/")
    .replace(/^\.\//, "")
    .replace(/^\/+|\/+$/g, "");
  const normalizedTarget = target.path
    .replaceAll("\\", "/")
    .replace(/^\/+|\/+$/g, "");
  if (!normalizedLabel || !normalizedTarget) return false;

  const caseInsensitive = /^[A-Za-z]:\//.test(normalizedTarget);
  const comparableLabel = caseInsensitive
    ? normalizedLabel.toLowerCase()
    : normalizedLabel;
  const comparableTarget = caseInsensitive
    ? normalizedTarget.toLowerCase()
    : normalizedTarget;
  return (
    comparableLabel === comparableTarget ||
    comparableTarget.endsWith(`/${comparableLabel}`)
  );
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
  let decodedPath: string;
  try {
    decodedPath = decodeURIComponent(rawPath);
  } catch {
    return { kind: "blocked", reason: "Link path is not valid UTF-8." };
  }

  const normalized = decodedPath.trim().replaceAll("\\", "/");
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
  if (drive) return { kind: "local", path: `${drive}/${joined}`, fragment };
  if (body.startsWith("/"))
    return { kind: "local", path: `/${joined}`, fragment };
  return { kind: "workspace", path: joined, fragment };
}

export function markdownStreamInterval(textLength: number): number {
  if (textLength >= 64 * 1024) return 100;
  if (textLength >= 16 * 1024) return 64;
  return 32;
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
  let fragment = hashIndex >= 0 ? value.slice(hashIndex + 1) || null : null;
  const queryIndex = withoutFragment.indexOf("?");
  let path =
    queryIndex >= 0 ? withoutFragment.slice(0, queryIndex) : withoutFragment;

  // Codex file links may encode a source line as `/path/file.ts:42`.
  // Convert that suffix to the same fragment shape used by detected paths.
  if (!fragment) {
    const lineReference = /^(.*):(\d+)(?::\d+)?$/.exec(path);
    if (lineReference?.[1]) {
      path = lineReference[1];
      fragment = `L${lineReference[2]}`;
    }
  }

  return {
    path,
    fragment,
  };
}

function workspaceBaseDirectory(path?: string | null): string[] {
  if (!path) return [];
  const normalized = path.replaceAll("\\", "/").replace(/^\/+/, "");
  const segments = normalized.split("/").filter(Boolean);
  return segments.slice(0, -1);
}

function localBasePath(
  path?: string | null,
): { drive: string | null; unixRoot: boolean } | null {
  if (!path) return null;
  const normalized = path.replaceAll("\\", "/");
  const drive = /^[A-Za-z]:/.exec(normalized)?.[0] ?? null;
  if (drive) return { drive, unixRoot: false };
  return normalized.startsWith("/")
    ? { drive: null, unixRoot: true }
    : null;
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
