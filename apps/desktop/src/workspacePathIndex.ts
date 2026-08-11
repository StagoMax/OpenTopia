export type WorkspacePathStatus = "known" | "missing" | "unknown";

export type WorkspacePathIndexOptions = {
  workspaceRoot: string | null;
  /** Lists file names in a workspace-relative directory ("" is the root). */
  listDirectory(directory: string): Promise<string[]>;
  /** Reads a text file after this index has constrained it to the workspace. */
  readTextFile?(path: string): Promise<string>;
  /** How long a "missing" answer is trusted before the directory is re-read. */
  missingRetryMs?: number;
  now?(): number;
};

type DirectoryRecord = {
  files: Set<string>;
  lowercase: Set<string>;
  fetchedAt: number;
  loading: boolean;
};

const DEFAULT_MISSING_RETRY_MS = 10_000;

/**
 * Tracks which of the paths mentioned in a conversation actually exist in the
 * workspace, so only real files are rendered as clickable links. Lookups are
 * one directory listing per directory, cached and shared between links.
 */
export class WorkspacePathIndex {
  private readonly directories = new Map<string, DirectoryRecord>();
  private readonly listeners = new Set<() => void>();
  private readonly options: WorkspacePathIndexOptions;

  constructor(options: WorkspacePathIndexOptions) {
    this.options = options;
  }

  status(path: string): WorkspacePathStatus {
    const relative = this.toRelative(path);
    if (!relative) return "missing";
    const record = this.directories.get(relative.directory);
    if (!record || record.loading) return "unknown";
    return record.files.has(relative.name) ||
      record.lowercase.has(relative.name.toLowerCase())
      ? "known"
      : "missing";
  }

  /** Returns the absolute on-disk path for a target inside this workspace. */
  absolutePath(path: string): string | null {
    return toWorkspaceAbsolutePath(path, this.options.workspaceRoot);
  }

  async readTextFile(path: string): Promise<string> {
    const relative = toWorkspaceRelativePath(path, this.options.workspaceRoot);
    if (!relative) throw new Error("File is outside the active workspace.");
    if (!this.options.readTextFile) {
      throw new Error("Workspace file reading is unavailable.");
    }
    return this.options.readTextFile(relative);
  }

  /** Subscribes to status changes for `path` and starts the lookup it needs. */
  watch(path: string, listener: () => void): () => void {
    this.listeners.add(listener);
    void this.ensure(path);
    return () => {
      this.listeners.delete(listener);
    };
  }

  private async ensure(path: string): Promise<void> {
    const relative = this.toRelative(path);
    if (!relative) return;
    const record = this.directories.get(relative.directory);
    if (record) {
      if (record.loading) return;
      if (this.status(path) === "known") return;
      const retryMs = this.options.missingRetryMs ?? DEFAULT_MISSING_RETRY_MS;
      if (this.now() - record.fetchedAt < retryMs) return;
    }

    this.directories.set(relative.directory, {
      files: record?.files ?? new Set(),
      lowercase: record?.lowercase ?? new Set(),
      fetchedAt: this.now(),
      loading: true,
    });

    let names: string[] = [];
    try {
      names = await this.options.listDirectory(relative.directory);
    } catch {
      names = [];
    }

    this.directories.set(relative.directory, {
      files: new Set(names),
      lowercase: new Set(names.map((name) => name.toLowerCase())),
      fetchedAt: this.now(),
      loading: false,
    });
    for (const listener of this.listeners) listener();
  }

  private toRelative(path: string): { directory: string; name: string } | null {
    const relative = toWorkspaceRelativePath(path, this.options.workspaceRoot);
    return relative ? splitDirectoryAndName(relative) : null;
  }

  private now(): number {
    return this.options.now?.() ?? Date.now();
  }
}

/**
 * Maps a mentioned path onto a workspace-relative path. Absolute paths outside
 * the workspace — and paths that climb above it — resolve to null so they are
 * never linked.
 */
export function toWorkspaceRelativePath(
  path: string,
  workspaceRoot: string | null,
): string | null {
  const normalized = path.trim().replaceAll("\\", "/");
  if (!normalized) return null;

  const absolute = /^([A-Za-z]:)?\//.test(normalized) && normalized !== "/";
  if (!absolute) return collapse(normalized);

  if (!workspaceRoot) return null;
  const root = workspaceRoot.replaceAll("\\", "/").replace(/\/+$/, "");
  const caseInsensitive = /^[A-Za-z]:/.test(root);
  const haystack = caseInsensitive ? normalized.toLowerCase() : normalized;
  const needle = caseInsensitive ? root.toLowerCase() : root;
  if (haystack !== needle && !haystack.startsWith(`${needle}/`)) return null;
  return collapse(normalized.slice(root.length));
}

/** Resolves a workspace-relative mention without allowing it to escape. */
export function toWorkspaceAbsolutePath(
  path: string,
  workspaceRoot: string | null,
): string | null {
  if (!workspaceRoot) return null;
  const relative = toWorkspaceRelativePath(path, workspaceRoot);
  if (!relative) return null;

  const root = workspaceRoot.replace(/[\\/]+$/, "");
  const separator = root.includes("\\") ? "\\" : "/";
  return `${root}${separator}${relative.replaceAll("/", separator)}`;
}

function splitDirectoryAndName(relative: string): {
  directory: string;
  name: string;
} {
  const index = relative.lastIndexOf("/");
  return index < 0
    ? { directory: "", name: relative }
    : { directory: relative.slice(0, index), name: relative.slice(index + 1) };
}

function collapse(path: string): string | null {
  const segments: string[] = [];
  for (const segment of path.split("/")) {
    if (!segment || segment === ".") continue;
    if (segment === "..") {
      if (segments.length === 0) return null;
      segments.pop();
      continue;
    }
    segments.push(segment);
  }
  return segments.length ? segments.join("/") : null;
}
