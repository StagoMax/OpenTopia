import type {
  ChangedFile,
  GitBranchInfo,
  ReviewFileRequest,
  WorkspaceDiff,
} from "../../types";
import {
  matchesPathQuery,
  parseUnifiedDiff,
  type DiffTreeNode,
  type ParsedDiffFile,
} from "../../diffReview";

export type DiffReviewFileContent = {
  content: string;
  truncated: boolean;
};

/** One recorded agent turn, offered as a review baseline. Newest first. */
export type DiffReviewTurnScope = {
  turnId: string;
  label: string;
  additions: number;
  deletions: number;
  files: Array<{ path: string; binary: boolean }>;
};

export type DiffReviewGitAction = "commit" | "commit_push" | "push";

export type DiffReviewPanelProps = {
  workspaceDiff: WorkspaceDiff | null;
  turnScopes: DiffReviewTurnScope[];
  /** File another surface asked to review; the nonce refocuses the same path. */
  focusRequest: ReviewFileRequest | null;
  isRefreshing: boolean;
  revertingPath: string | null;
  canRunGit: boolean;
  onRefresh(): void;
  onOpenFileTab(path: string): void;
  onLoadFileContent(path: string): Promise<DiffReviewFileContent>;
  onLoadTurnFileDiff(turnId: string, path: string): Promise<string>;
  onRevertFile(path: string): void;
  /** Why this path cannot be restored, or null when it can. */
  revertBlockedReason(path: string): string | null;
  onGitAction(action: DiffReviewGitAction, message: string): Promise<string>;
  onListGitBranches(): Promise<GitBranchInfo[]>;
  onSwitchGitBranch(branch: string): Promise<void>;
};

export const workspaceScopeId = "workspace";
export const defaultRowLimit = 800;
export const initialRenderedFileCount = 1;
export const turnDiffConcurrency = 3;

export type ReviewScope =
  | { id: "workspace"; kind: "workspace"; label: string }
  | { id: string; kind: "turn"; label: string; turn: DiffReviewTurnScope };

export type ContentState = {
  status: "loading" | "ready" | "error";
  lines?: string[];
  text?: string;
  truncated?: boolean;
  error?: string;
};

export type DiffSplitPane = "left" | "right";

export type TurnFilesState = {
  status: "loading" | "ready" | "error";
  files: ParsedDiffFile[];
  loadedFileCount: number;
  totalFileCount: number;
  error?: string;
};

export function filterTree(
  nodes: DiffTreeNode[],
  query: string,
): DiffTreeNode[] {
  if (!query.trim()) return nodes;
  const result: DiffTreeNode[] = [];
  for (const node of nodes) {
    if (node.type === "file") {
      if (matchesPathQuery(node.path, query)) result.push(node);
      continue;
    }
    const children = filterTree(node.children, query);
    if (children.length) result.push({ ...node, children });
  }
  return result;
}

export function buildWorkspaceFiles(
  diff: WorkspaceDiff | null,
): ParsedDiffFile[] {
  if (!diff) return [];
  const combined = diff.diff?.trim()
    ? diff.diff
    : [diff.stagedDiff ?? "", diff.unstagedDiff ?? ""]
        .filter((text) => text.trim())
        .join("\n");
  const files = parseUnifiedDiff(combined);
  const seen = new Set(files.map((file) => normalizePath(file.path)));
  // Untracked files never appear in `git diff`, but the reader still expects
  // to see them listed with the rest of the change.
  for (const changed of diff.files) {
    if (seen.has(normalizePath(changed.path))) continue;
    seen.add(normalizePath(changed.path));
    files.push(emptyPlaceholderFile(changed.path, changedFileStatus(changed)));
  }
  return files;
}

function changedFileStatus(file: ChangedFile): ParsedDiffFile["status"] {
  if (file.isUntracked || file.status === "??") return "added";
  if (file.isRenamed || file.originalPath) return "renamed";
  return "modified";
}

export function emptyPlaceholderFile(
  path: string,
  status: ParsedDiffFile["status"] = "modified",
): ParsedDiffFile {
  return {
    path,
    oldPath: status === "added" ? null : path,
    newPath: path,
    status,
    binary: false,
    additions: 0,
    deletions: 0,
    hunks: [],
    patch: "",
  };
}

export function binaryPlaceholderFile(path: string): ParsedDiffFile {
  return { ...emptyPlaceholderFile(path), binary: true };
}

/** Turns a loaded file into an all-added hunk so untracked files can be read. */
export function withLoadedContent(
  file: ParsedDiffFile,
  lines: string[],
): ParsedDiffFile {
  if (!lines.length) return file;
  return {
    ...file,
    hunks: [
      {
        header: `@@ -0,0 +1,${lines.length} @@`,
        oldStart: 0,
        oldLines: 0,
        newStart: 1,
        newLines: lines.length,
        lines: lines.map((text, index) => ({
          kind: "added" as const,
          oldLine: null,
          newLine: index + 1,
          text,
        })),
        patch: "",
      },
    ],
  };
}

export function localGapIds(
  expanded: ReadonlySet<string>,
  path: string,
): ReadonlySet<string> {
  const prefix = `${path}::`;
  const ids = new Set<string>();
  for (const key of expanded) {
    if (key.startsWith(prefix)) ids.add(key.slice(prefix.length));
  }
  return ids;
}

export function statusLabel(file: ParsedDiffFile): string {
  switch (file.status) {
    case "added":
      return "新增";
    case "deleted":
      return "删除";
    case "renamed":
      return "重命名";
    default:
      return "修改";
  }
}

export function isRichPreviewPath(path: string): boolean {
  return /\.(md|markdown|mdx)$/i.test(path);
}

function normalizePath(path: string): string {
  return path.replace(/\\/g, "/");
}

export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
