import {
  useCallback,
  useEffect,
  useMemo,
  useState,
  type CSSProperties,
} from "react";
import {
  ChevronDown,
  ChevronRight,
  ExternalLink,
  FileText,
  Folder,
  FolderOpen,
  Loader2,
  RefreshCw,
  Search,
} from "lucide-react";
import type { ApiClient } from "../../api/client";
import type {
  ContextStatus,
  WorkspaceEntry,
  WorkspaceFilePreview,
  WorkspaceTree,
} from "../../types";
import { detectLanguage, MonacoEditor } from "../MonacoEditor";
import { Badge, Button, IconButton } from "../ui";
import {
  formatBytes,
  formatNumber,
  splitWorkspacePath,
  toWorkspaceAbsolutePath,
} from "./workbenchFormat";

export function ContextCard({
  contextStatus,
  disabled,
  isCompacting,
  onCompactContext,
}: {
  contextStatus: ContextStatus | null;
  disabled: boolean;
  isCompacting: boolean;
  onCompactContext(): void;
}) {
  const budget = contextStatus?.budget;
  const usage = budget?.estimatedUsage ?? 0;
  const latestSummary = contextStatus?.latestSummary;
  const providerUsage = contextStatus?.usage;
  const projection = contextStatus?.projection;

  return (
    <section className="panel-card context-card">
      <div className="panel-title">
        <FileText size={16} />
        Context
      </div>
      <div className="context-budget-row">
        <span>{budget ? `${usage}% used` : "No estimate"}</span>
        <span>{budget ? `${budget.messageCount} messages` : "No thread"}</span>
        {budget && <span>{formatNumber(budget.usedTokens)} tokens</span>}
      </div>
      {budget && (
        <div className="context-meter" aria-label="Context usage">
          <span style={{ width: `${Math.min(usage, 100)}%` }} />
        </div>
      )}
      {providerUsage && providerUsage.modelRequests > 0 ? (
        <div className="context-budget-row">
          <span>{providerUsage.modelRequests} requests</span>
          <span>{formatNumber(providerUsage.cachedInputTokens)} cached</span>
          {providerUsage.cacheWriteTokens > 0 ? (
            <span>{formatNumber(providerUsage.cacheWriteTokens)} written</span>
          ) : null}
          {providerUsage.compactions > 0 ? (
            <span>{providerUsage.compactions} compactions</span>
          ) : null}
          {providerUsage.lastFactRetentionPercent > 0 ? (
            <span>
              {providerUsage.lastFactRetentionPercent}% facts retained
            </span>
          ) : null}
          {providerUsage.nativeCompactions > 0 ? (
            <span>{providerUsage.nativeCompactions} native</span>
          ) : null}
          {providerUsage.providerFallbacks > 0 ? (
            <span>{providerUsage.providerFallbacks} fallbacks</span>
          ) : null}
          {providerUsage.warnings > 0 ? (
            <span>{providerUsage.warnings} warnings</span>
          ) : null}
        </div>
      ) : null}
      {projection ? (
        <div className="context-budget-row">
          <Badge variant="info">
            {(projection.checkpointMode ?? "uncompacted").replaceAll("_", " ")}
          </Badge>
          <Badge
            variant={projection.providerStateAvailable ? "success" : "neutral"}
          >
            {projection.providerStateAvailable
              ? (projection.providerStateKind ?? "provider state").replaceAll(
                  "_",
                  " ",
                )
              : projection.nativeCompactionSupported
                ? "native ready"
                : "local checkpoint"}
          </Badge>
          <Badge>{formatNumber(projection.checkpointTokens)} checkpoint</Badge>
          <Badge>{formatNumber(projection.recentTailTokens)} recent</Badge>
          {projection.unsummarizedMessageCount > 0 ? (
            <Badge variant="warning">
              {projection.unsummarizedMessageCount} messages uncovered
            </Badge>
          ) : null}
          {projection.unsummarizedEventCount > 0 ? (
            <Badge variant="warning">
              {projection.unsummarizedEventCount} events uncovered
            </Badge>
          ) : null}
        </div>
      ) : null}
      {latestSummary ? (
        <details className="context-summary">
          <summary>
            Checkpoint through event {latestSummary.coveredThroughSeq}
            <ChevronDown size={12} />
          </summary>
          <p>{latestSummary.checkpoint?.goal ?? latestSummary.summary}</p>
        </details>
      ) : (
        <p>No context checkpoint yet.</p>
      )}
      <Button
        size="compact"
        variant="secondary"
        disabled={disabled}
        onClick={onCompactContext}
      >
        <RefreshCw size={13} className={isCompacting ? "spin" : ""} />
        {isCompacting ? "Compacting" : "Compact"}
      </Button>
    </section>
  );
}

export function FilesView({
  client,
  threadId,
  workspaceRoot,
  workspaceTree,
  filePreview,
  isRefreshing,
  onRefresh,
  onOpenWorkspacePath,
  onOpenWorkspaceEntry,
  onOpenPath,
}: {
  client: ApiClient | null;
  threadId: string | null;
  workspaceRoot: string | null;
  workspaceTree: WorkspaceTree | null;
  filePreview: WorkspaceFilePreview | null;
  isRefreshing: boolean;
  onRefresh(): void;
  onOpenWorkspacePath(path?: string): void;
  onOpenWorkspaceEntry(entry: WorkspaceEntry): void;
  onOpenPath(targetPath: string): void;
}) {
  const currentPath = workspaceTree?.path ?? "";
  const entries = workspaceTree?.entries ?? [];
  const [query, setQuery] = useState("");
  const [expandedPaths, setExpandedPaths] = useState<ReadonlySet<string>>(
    new Set(),
  );
  const [childrenByPath, setChildrenByPath] = useState<
    Readonly<Record<string, WorkspaceEntry[]>>
  >({});
  const [loadingPaths, setLoadingPaths] = useState<ReadonlySet<string>>(
    new Set(),
  );
  const [treeError, setTreeError] = useState<string | null>(null);
  const normalizedQuery = query.trim().toLocaleLowerCase();
  const treeRows = useMemo(
    () => flattenWorkspaceTree(entries, expandedPaths, childrenByPath),
    [childrenByPath, entries, expandedPaths],
  );
  const visibleRows = normalizedQuery
    ? treeRows.filter(({ entry }) =>
        `${entry.name} ${entry.path}`
          .toLocaleLowerCase()
          .includes(normalizedQuery),
      )
    : treeRows;

  useEffect(() => {
    setExpandedPaths(new Set());
    setChildrenByPath({});
    setLoadingPaths(new Set());
    setTreeError(null);
  }, [currentPath, threadId]);

  const toggleDirectory = useCallback(
    async (entry: WorkspaceEntry) => {
      if (entry.kind !== "directory") return;
      if (expandedPaths.has(entry.path)) {
        setExpandedPaths((current) => {
          const next = new Set(current);
          next.delete(entry.path);
          return next;
        });
        return;
      }

      setExpandedPaths((current) => new Set(current).add(entry.path));
      if (childrenByPath[entry.path] || !client || !threadId) return;

      setLoadingPaths((current) => new Set(current).add(entry.path));
      setTreeError(null);
      try {
        const tree = await client.listWorkspaceTree(threadId, entry.path);
        setChildrenByPath((current) => ({
          ...current,
          [entry.path]: tree.entries,
        }));
      } catch (cause) {
        setExpandedPaths((current) => {
          const next = new Set(current);
          next.delete(entry.path);
          return next;
        });
        setTreeError(errorMessage(cause));
      } finally {
        setLoadingPaths((current) => {
          const next = new Set(current);
          next.delete(entry.path);
          return next;
        });
      }
    },
    [childrenByPath, client, expandedPaths, threadId],
  );

  const refreshTree = useCallback(() => {
    setExpandedPaths(new Set());
    setChildrenByPath({});
    setLoadingPaths(new Set());
    setTreeError(null);
    onRefresh();
  }, [onRefresh]);

  return (
    <div className="files-view">
      <div className="file-browser-toolbar">
        <Breadcrumb
          path={filePreview?.path ?? currentPath}
          rootLabel={workspaceRoot ? workspaceName(workspaceRoot) : "工作区"}
          leafIsFile={Boolean(filePreview)}
          onOpenPath={onOpenWorkspacePath}
        />
        {filePreview?.truncated ? (
          <Badge variant="warning">已截断</Badge>
        ) : null}
        {filePreview?.readonly ? <Badge>只读</Badge> : null}
        <IconButton
          size="compact"
          variant="quiet"
          aria-label="刷新文件"
          title="刷新文件"
          disabled={isRefreshing}
          onClick={refreshTree}
        >
          <RefreshCw size={13} className={isRefreshing ? "spin" : ""} />
        </IconButton>
        <IconButton
          size="compact"
          variant="quiet"
          aria-label="在系统中打开工作区"
          title="在系统中打开工作区"
          disabled={!workspaceRoot}
          onClick={() => workspaceRoot && onOpenPath(workspaceRoot)}
        >
          <FolderOpen size={14} aria-hidden="true" />
        </IconButton>
        {filePreview ? (
          <Button
            className="file-browser-open"
            size="compact"
            variant="secondary"
            disabled={!workspaceRoot}
            title={`打开 ${filePreview.path}`}
            onClick={() =>
              workspaceRoot &&
              onOpenPath(
                toWorkspaceAbsolutePath(workspaceRoot, filePreview.path),
              )
            }
          >
            <ExternalLink size={14} aria-hidden="true" />
            <span>打开</span>
          </Button>
        ) : null}
      </div>

      <div className="file-browser-layout">
        <section className="file-preview-workspace" aria-label="文件预览">
          {filePreview ? (
            <div className="workbench-file-editor">
              <MonacoEditor
                value={filePreview.content}
                language={detectLanguage(filePreview.path)}
                readOnly
                theme={
                  document.documentElement.dataset.theme === "dark"
                    ? "vs-dark"
                    : "vs"
                }
              />
            </div>
          ) : (
            <div className="file-preview-empty">
              <FolderOpen size={32} aria-hidden="true" />
              <strong>打开文件</strong>
              <span>从工作区目录树中选择文件</span>
            </div>
          )}
        </section>

        <aside className="file-explorer-pane" aria-label="工作区目录树">
          <label className="file-explorer-search">
            <Search size={14} aria-hidden="true" />
            <span className="sr-only">筛选文件</span>
            <input
              value={query}
              placeholder="筛选文件..."
              onChange={(event) => setQuery(event.target.value)}
            />
          </label>
          <div className="workbench-file-list">
            {visibleRows.length ? (
              visibleRows.map(({ entry, depth }) => {
                const isDirectory = entry.kind === "directory";
                const expanded = expandedPaths.has(entry.path);
                const loading = loadingPaths.has(entry.path);
                return (
                  <button
                    className={`file-row workbench-file-row ${
                      filePreview?.path === entry.path ? "active" : ""
                    }`}
                    key={entry.path}
                    type="button"
                    title={entry.path}
                    aria-expanded={isDirectory ? expanded : undefined}
                    style={{ "--file-tree-depth": depth } as CSSProperties}
                    onClick={() => {
                      if (isDirectory) void toggleDirectory(entry);
                      else onOpenWorkspaceEntry(entry);
                    }}
                  >
                    {loading ? (
                      <Loader2 className="spin" size={14} aria-hidden="true" />
                    ) : isDirectory ? (
                      <ChevronRight
                        className={expanded ? "is-expanded" : ""}
                        size={14}
                        aria-hidden="true"
                      />
                    ) : (
                      <span className="file-explorer-indent" />
                    )}
                    {isDirectory ? (
                      <Folder size={14} aria-hidden="true" />
                    ) : (
                      <FileText size={14} aria-hidden="true" />
                    )}
                    <span>{entry.name}</span>
                    <small>{isDirectory ? "" : formatBytes(entry.size)}</small>
                  </button>
                );
              })
            ) : (
              <span className="file-explorer-empty">
                {entries.length ? "没有匹配的文件。" : "当前目录为空。"}
              </span>
            )}
          </div>
          {treeError ? (
            <p className="file-explorer-error" role="alert">
              {treeError}
            </p>
          ) : null}
        </aside>
      </div>
    </div>
  );
}

function Breadcrumb({
  path,
  rootLabel,
  leafIsFile = false,
  onOpenPath,
}: {
  path: string;
  rootLabel: string;
  leafIsFile?: boolean;
  onOpenPath(path?: string): void;
}) {
  const parts = splitWorkspacePath(path);

  return (
    <nav className="path-breadcrumb" aria-label="Current folder">
      <button type="button" onClick={() => onOpenPath(undefined)}>
        {rootLabel}
      </button>
      {parts.map((part, index) => {
        const partPath = parts.slice(0, index + 1).join("/");
        const isFileLeaf = leafIsFile && index === parts.length - 1;
        return (
          <span key={`${part}-${index}`} className="breadcrumb-part">
            <ChevronRight size={12} aria-hidden="true" />
            {isFileLeaf ? (
              <strong title={partPath}>{part}</strong>
            ) : (
              <button
                type="button"
                title={partPath}
                onClick={() => onOpenPath(partPath)}
              >
                {part}
              </button>
            )}
          </span>
        );
      })}
    </nav>
  );
}

function workspaceName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).at(-1) ?? path;
}

type WorkspaceTreeRow = {
  entry: WorkspaceEntry;
  depth: number;
};

function flattenWorkspaceTree(
  entries: WorkspaceEntry[],
  expandedPaths: ReadonlySet<string>,
  childrenByPath: Readonly<Record<string, WorkspaceEntry[]>>,
  depth = 0,
): WorkspaceTreeRow[] {
  const rows: WorkspaceTreeRow[] = [];
  for (const entry of entries) {
    rows.push({ entry, depth });
    if (entry.kind !== "directory" || !expandedPaths.has(entry.path)) continue;
    rows.push(
      ...flattenWorkspaceTree(
        childrenByPath[entry.path] ?? [],
        expandedPaths,
        childrenByPath,
        depth + 1,
      ),
    );
  }
  return rows;
}

function errorMessage(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}
