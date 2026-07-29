import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  AlertCircle,
  Check,
  ChevronDown,
  ChevronRight,
  Columns2,
  Copy,
  ExternalLink,
  FileCode2,
  FileText,
  FoldVertical,
  Folder,
  GitCommitHorizontal,
  Loader2,
  MoreHorizontal,
  PanelRight,
  RefreshCw,
  RotateCcw,
  Rows3,
  Search,
  UnfoldVertical,
} from "lucide-react";
import type { ChangedFile, ReviewFileRequest, WorkspaceDiff } from "../types";
import {
  buildDiffBlocks,
  buildDiffFileTree,
  buildGitApplyCommand,
  buildSplitRows,
  buildUnifiedRows,
  countDiffRows,
  diffFileDirectory,
  diffFileName,
  diffLanguageFromPath,
  fileAdditions,
  fileDeletions,
  matchesPathQuery,
  parseUnifiedDiff,
  splitFileContent,
  summarizeDiffStats,
  type DiffBuildOptions,
  type DiffRowSide,
  type DiffSpan,
  type DiffSplitRow,
  type DiffTreeNode,
  type DiffUnifiedRow,
  type ParsedDiffFile,
} from "../diffReview";
import {
  readDiffReviewPreferences,
  writeDiffReviewPreferences,
  type DiffReviewPreferences,
} from "../diffReviewPreferences";
import { MarkdownContent } from "./MarkdownContent";
import { Button, IconButton, Popover } from "./ui";
import "./DiffReviewPanel.css";

export type DiffReviewFileContent = { content: string; truncated: boolean };

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
};

const workspaceScopeId = "workspace";
const defaultRowLimit = 800;
const initialRenderedFileCount = 1;
const turnDiffConcurrency = 3;

type ReviewScope =
  | { id: "workspace"; kind: "workspace"; label: string }
  | { id: string; kind: "turn"; label: string; turn: DiffReviewTurnScope };

type ContentState = {
  status: "loading" | "ready" | "error";
  lines?: string[];
  text?: string;
  truncated?: boolean;
  error?: string;
};

type TurnFilesState = {
  status: "loading" | "ready" | "error";
  files: ParsedDiffFile[];
  loadedFileCount: number;
  totalFileCount: number;
  error?: string;
};

export function DiffReviewPanel({
  workspaceDiff,
  turnScopes,
  focusRequest,
  isRefreshing,
  revertingPath,
  canRunGit,
  onRefresh,
  onOpenFileTab,
  onLoadFileContent,
  onLoadTurnFileDiff,
  onRevertFile,
  revertBlockedReason,
  onGitAction,
}: DiffReviewPanelProps) {
  const [preferences, setPreferences] = useState<DiffReviewPreferences>(
    readDiffReviewPreferences,
  );
  const [scopeId, setScopeId] = useState<string>(
    focusRequest
      ? workspaceScopeId
      : turnScopes[0]
        ? `turn:${turnScopes[0].turnId}`
        : workspaceScopeId,
  );
  const [collapsedFiles, setCollapsedFiles] = useState<ReadonlySet<string>>(
    new Set(),
  );
  const [expandedGaps, setExpandedGaps] = useState<ReadonlySet<string>>(
    new Set(),
  );
  const [contents, setContents] = useState<Record<string, ContentState>>({});
  const [turnFiles, setTurnFiles] = useState<Record<string, TurnFilesState>>(
    {},
  );
  const [rowLimits, setRowLimits] = useState<Record<string, number>>({});
  const [renderedPaths, setRenderedPaths] = useState<ReadonlySet<string>>(
    new Set(),
  );
  const [activePath, setActivePath] = useState<string | null>(null);
  const [focusPath, setFocusPath] = useState<string | null>(null);
  const [treeFilter, setTreeFilter] = useState("");
  const [notice, setNotice] = useState<string | null>(null);
  const sectionRefs = useRef(new Map<string, HTMLElement>());
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const requestedPaths = useRef(new Set<string>());
  const renderedScopeRef = useRef<string | null>(null);

  useEffect(() => writeDiffReviewPreferences(preferences), [preferences]);

  const update = useCallback((patch: Partial<DiffReviewPreferences>) => {
    setPreferences((current) => ({ ...current, ...patch }));
  }, []);

  const scopes = useMemo<ReviewScope[]>(
    () => [
      ...turnScopes.map((turn, index) => ({
        id: `turn:${turn.turnId}`,
        kind: "turn" as const,
        label: index === 0 ? "上一轮" : turn.label,
        turn,
      })),
      {
        id: workspaceScopeId,
        kind: "workspace" as const,
        label: "工作区全部改动",
      },
    ],
    [turnScopes],
  );

  const scope =
    scopes.find((candidate) => candidate.id === scopeId) ??
    scopes.at(-1) ??
    null;

  // Until the reader picks a baseline, the panel follows the newest recorded
  // turn — which is usually what "review my changes" means, and which is not
  // known yet when the panel mounts ahead of the event stream.
  const scopePickedRef = useRef(false);
  const selectScope = useCallback((id: string) => {
    scopePickedRef.current = true;
    setScopeId(id);
  }, []);

  useEffect(() => {
    if (scopePickedRef.current || !turnScopes[0]) return;
    setScopeId(`turn:${turnScopes[0].turnId}`);
  }, [turnScopes]);

  const workspaceFiles = useMemo(
    () => buildWorkspaceFiles(workspaceDiff),
    [workspaceDiff],
  );

  // A refreshed diff describes different file contents, so anything already
  // read from disk is stale and must be fetched again on demand.
  useEffect(() => {
    requestedPaths.current.clear();
    setContents({});
  }, [workspaceDiff]);

  const loadTurnFiles = useCallback(
    (turn: DiffReviewTurnScope) => {
      const totalFileCount = turn.files.length;
      setTurnFiles((current) =>
        current[turn.turnId]?.status === "loading"
          ? current
          : {
              ...current,
              [turn.turnId]: {
                status: "loading",
                files: [],
                loadedFileCount: 0,
                totalFileCount,
              },
            },
      );
      if (!totalFileCount) return;

      const results: Array<ParsedDiffFile[] | null> = Array.from(
        { length: totalFileCount },
        () => null,
      );
      let nextIndex = 0;
      let loadedFileCount = 0;
      let firstError: string | undefined;

      const publish = () => {
        setTurnFiles((current) => {
          const state = current[turn.turnId];
          if (!state || state.status === "error") return current;
          return {
            ...current,
            [turn.turnId]: {
              status: loadedFileCount === totalFileCount ? "ready" : "loading",
              files: results.flatMap((result) => result ?? []),
              loadedFileCount,
              totalFileCount,
              error: firstError,
            },
          };
        });
      };

      const loadNext = async () => {
        while (nextIndex < totalFileCount) {
          const index = nextIndex;
          nextIndex += 1;
          const file = turn.files[index];
          try {
            if (file.binary) {
              results[index] = [binaryPlaceholderFile(file.path)];
            } else {
              const diff = await onLoadTurnFileDiff(turn.turnId, file.path);
              const parsed = parseUnifiedDiff(diff, file.path);
              results[index] = parsed.length
                ? parsed
                : [emptyPlaceholderFile(file.path)];
            }
          } catch (error: unknown) {
            firstError ??= errorMessage(error);
            results[index] = [emptyPlaceholderFile(file.path)];
          }
          loadedFileCount += 1;
          publish();
        }
      };

      void Promise.all(
        Array.from(
          { length: Math.min(turnDiffConcurrency, totalFileCount) },
          () => loadNext(),
        ),
      );
    },
    [onLoadTurnFileDiff],
  );

  useEffect(() => {
    if (scope?.kind !== "turn") return;
    if (turnFiles[scope.turn.turnId]) return;
    loadTurnFiles(scope.turn);
  }, [loadTurnFiles, scope, turnFiles]);

  const turnState =
    scope?.kind === "turn" ? turnFiles[scope.turn.turnId] : null;
  const files =
    scope?.kind === "turn" ? (turnState?.files ?? []) : workspaceFiles;
  const stats = useMemo(() => summarizeDiffStats(files), [files]);

  const requestContent = useCallback(
    (path: string) => {
      if (requestedPaths.current.has(path)) return;
      requestedPaths.current.add(path);
      setContents((current) => ({ ...current, [path]: { status: "loading" } }));
      void onLoadFileContent(path)
        .then((preview) =>
          setContents((next) => ({
            ...next,
            [path]: {
              status: "ready",
              lines: splitFileContent(preview.content),
              text: preview.content,
              truncated: preview.truncated,
            },
          })),
        )
        .catch((error: unknown) =>
          setContents((next) => ({
            ...next,
            [path]: { status: "error", error: errorMessage(error) },
          })),
        );
    },
    [onLoadFileContent],
  );

  // "Load full file" and the rich preview both need the working-tree file, so
  // turning either on fetches every file the reader can currently see.
  useEffect(() => {
    if (!preferences.loadFullFile && !preferences.richPreview) return;
    for (const file of files) {
      if (file.binary || file.status === "deleted") continue;
      if (preferences.richPreview && !preferences.loadFullFile) {
        if (!isRichPreviewPath(file.path)) continue;
      }
      requestContent(file.path);
    }
  }, [
    files,
    preferences.loadFullFile,
    preferences.richPreview,
    requestContent,
  ]);

  useEffect(() => {
    const scopeKey = scope?.id ?? null;
    const initialPaths = files
      .slice(0, initialRenderedFileCount)
      .map((file) => file.path);
    setRenderedPaths((current) => {
      if (renderedScopeRef.current !== scopeKey) {
        renderedScopeRef.current = scopeKey;
        return new Set(initialPaths);
      }
      const next = new Set(current);
      let changed = false;
      for (const path of initialPaths) {
        if (next.has(path)) continue;
        next.add(path);
        changed = true;
      }
      return changed ? next : current;
    });
  }, [files, scope?.id]);

  const renderFile = useCallback((path: string) => {
    setRenderedPaths((current) => {
      if (current.has(path)) return current;
      const next = new Set(current);
      next.add(path);
      return next;
    });
  }, []);

  useEffect(() => {
    const root = scrollRef.current;
    if (!root) return undefined;
    const observer = new IntersectionObserver(
      (entries) => {
        const visible = entries
          .filter((entry) => entry.isIntersecting)
          .sort(
            (left, right) =>
              left.boundingClientRect.top - right.boundingClientRect.top,
          );
        const path = visible[0]?.target.getAttribute("data-file-path");
        if (path) setActivePath((current) => (current === path ? current : path));
        setRenderedPaths((current) => {
          const next = new Set(current);
          let changed = false;
          for (const entry of visible) {
            const visiblePath = entry.target.getAttribute("data-file-path");
            if (!visiblePath || next.has(visiblePath)) continue;
            next.add(visiblePath);
            changed = true;
          }
          return changed ? next : current;
        });
      },
      { root, rootMargin: "0px 0px -60% 0px", threshold: 0 },
    );
    for (const element of sectionRefs.current.values())
      observer.observe(element);
    return () => observer.disconnect();
  }, [files]);

  const allCollapsed =
    files.length > 0 && files.every((file) => collapsedFiles.has(file.path));

  const toggleFile = useCallback((path: string) => {
    renderFile(path);
    setCollapsedFiles((current) => {
      const next = new Set(current);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }, [renderFile]);

  const toggleAllFiles = useCallback(() => {
    setCollapsedFiles((current) => {
      const currentPaths = files.map((file) => file.path);
      const collapse = currentPaths.some((path) => !current.has(path));
      const next = new Set(current);
      for (const path of currentPaths) {
        if (collapse) next.add(path);
        else next.delete(path);
      }
      return next;
    });
  }, [files]);

  const expandGap = useCallback(
    (path: string, gapId: string) => {
      requestContent(path);
      setExpandedGaps((current) => new Set(current).add(`${path}::${gapId}`));
    },
    [requestContent],
  );

  const scrollToFile = useCallback((path: string) => {
    renderFile(path);
    setCollapsedFiles((current) => {
      if (!current.has(path)) return current;
      const next = new Set(current);
      next.delete(path);
      return next;
    });
    setActivePath(path);
    // The section may have just been expanded, so wait for the paint.
    requestAnimationFrame(() =>
      sectionRefs.current
        .get(path)
        ?.scrollIntoView({ block: "start", behavior: "smooth" }),
    );
  }, [renderFile]);

  // A "review this file" request names a working-tree path, so it always
  // resolves against the workspace baseline rather than a recorded turn.
  useEffect(() => {
    if (!focusRequest) return;
    selectScope(workspaceScopeId);
    setFocusPath(focusRequest.path);
  }, [focusRequest, selectScope]);

  useEffect(() => {
    if (!focusPath) return;
    const element = sectionRefs.current.get(focusPath);
    // The section may not be mounted yet; this effect reruns when it is.
    if (!element) return;
    setCollapsedFiles((current) => {
      if (!current.has(focusPath)) return current;
      const next = new Set(current);
      next.delete(focusPath);
      return next;
    });
    setActivePath(focusPath);
    renderFile(focusPath);
    element.scrollIntoView({ block: "start", behavior: "smooth" });
    setFocusPath(null);
  }, [files, focusPath, renderFile]);

  const patchText = useMemo(
    () =>
      files
        .map((file) => file.patch)
        .filter(Boolean)
        .join("\n"),
    [files],
  );

  const copyGitApply = useCallback(() => {
    const command = buildGitApplyCommand(
      patchText,
      navigator.userAgent.toLocaleLowerCase().includes("windows")
        ? "powershell"
        : "posix",
    );
    if (!command) return;
    void navigator.clipboard
      .writeText(command)
      .then(() => setNotice("已复制 git apply 命令"))
      .catch((error: unknown) => setNotice(errorMessage(error)));
  }, [patchText]);

  useEffect(() => {
    if (!notice) return undefined;
    const timer = setTimeout(() => setNotice(null), 4000);
    return () => clearTimeout(timer);
  }, [notice]);

  const tree = useMemo(() => buildDiffFileTree(files), [files]);
  const filteredTree = useMemo(
    () => filterTree(tree, treeFilter),
    [tree, treeFilter],
  );

  // Rebuilding this object on every render would invalidate every file's row
  // memo, which is the expensive part of the panel.
  const buildOptions = useMemo<DiffBuildOptions>(
    () => ({
      ignoreWhitespace: preferences.hideWhitespace,
      wordDiff: preferences.wordDiff,
    }),
    [preferences.hideWhitespace, preferences.wordDiff],
  );

  return (
    <div
      className="diff-review"
      data-view={preferences.view}
      data-wrap={preferences.wrapLines ? "on" : "off"}
    >
      <header className="diff-review__toolbar">
        <ScopePicker
          scopes={scopes}
          activeId={scope?.id ?? workspaceScopeId}
          onSelect={selectScope}
        />
        <span
          className="diff-review__stats"
          aria-label={`增加 ${stats.additions} 行，删除 ${stats.deletions} 行`}
        >
          <span className="is-addition">+{stats.additions}</span>
          <span className="is-deletion">-{stats.deletions}</span>
        </span>
        {workspaceDiff?.truncated && scope?.kind === "workspace" ? (
          <span className="diff-review__pill">差异已截断</span>
        ) : null}
        <span className="diff-review__spacer" />
        {notice ? (
          <span className="diff-review__notice" role="status">
            {notice}
          </span>
        ) : null}
        <OptionsMenu
          preferences={preferences}
          isRefreshing={isRefreshing}
          canCopyPatch={Boolean(buildGitApplyCommand(patchText))}
          onUpdate={update}
          onRefresh={onRefresh}
          onCopyGitApply={copyGitApply}
        />
        <IconButton
          aria-label={allCollapsed ? "展开全部差异" : "折叠全部差异"}
          title={allCollapsed ? "展开全部差异" : "折叠全部差异"}
          size="compact"
          disabled={!files.length}
          onClick={toggleAllFiles}
        >
          {allCollapsed ? (
            <UnfoldVertical size={14} />
          ) : (
            <FoldVertical size={14} />
          )}
        </IconButton>
        <JumpToFilePicker files={files} onSelect={scrollToFile} />
        <IconButton
          aria-label={
            preferences.view === "split"
              ? "切换到统一差异视图"
              : "切换到拆分差异视图"
          }
          title={
            preferences.view === "split"
              ? "切换到统一差异视图"
              : "切换到拆分差异视图"
          }
          size="compact"
          onClick={() =>
            update({ view: preferences.view === "split" ? "unified" : "split" })
          }
        >
          {preferences.view === "split" ? (
            <Rows3 size={14} />
          ) : (
            <Columns2 size={14} />
          )}
        </IconButton>
        <IconButton
          aria-label={preferences.showFilePanel ? "隐藏文件" : "显示文件"}
          title={preferences.showFilePanel ? "隐藏文件" : "显示文件"}
          size="compact"
          aria-pressed={preferences.showFilePanel}
          onClick={() => update({ showFilePanel: !preferences.showFilePanel })}
        >
          <PanelRight size={14} />
        </IconButton>
        <CommitMenu
          canRunGit={canRunGit}
          changedFiles={files.length}
          onGitAction={onGitAction}
        />
      </header>

      <div className="diff-review__body">
        <div className="diff-review__scroll" ref={scrollRef}>
          {scope?.kind === "turn" &&
          turnState?.status === "loading" &&
          files.length === 0 ? (
            <p className="diff-review__empty">
              <Loader2 className="spin" size={15} aria-hidden="true" />
              正在加载本轮差异…
            </p>
          ) : scope?.kind === "turn" &&
            turnState?.status === "error" &&
            files.length === 0 ? (
            <p className="diff-review__empty is-error">
              <AlertCircle size={15} aria-hidden="true" />
              {turnState.error}
            </p>
          ) : files.length === 0 ? (
            <p className="diff-review__empty">没有需要审阅的改动。</p>
          ) : (
            <>
              {scope?.kind === "turn" && turnState?.status === "loading" ? (
                <p className="diff-review__empty compact" role="status">
                  <Loader2 className="spin" size={14} aria-hidden="true" />
                  正在加载 {turnState.loadedFileCount}/{turnState.totalFileCount} 个文件…
                </p>
              ) : null}
              {turnState?.error ? (
                <p className="diff-review__empty compact is-error" role="status">
                  <AlertCircle size={14} aria-hidden="true" />
                  {turnState.error}
                </p>
              ) : null}
              {files.map((file) => (
                <DeferredDiffFileSection
                  key={file.path}
                  file={file}
                  content={contents[file.path] ?? null}
                  collapsed={collapsedFiles.has(file.path)}
                  active={activePath === file.path}
                  renderBody={renderedPaths.has(file.path)}
                  preferences={preferences}
                  buildOptions={buildOptions}
                  expandedGaps={expandedGaps}
                  rowLimit={rowLimits[file.path] ?? defaultRowLimit}
                  isReverting={revertingPath === file.path}
                  revertBlockedReason={
                    scope?.kind === "workspace"
                      ? revertBlockedReason(file.path)
                      : "只有工作区改动可以还原。"
                  }
                  registerSection={(element) => {
                    if (element) sectionRefs.current.set(file.path, element);
                    else sectionRefs.current.delete(file.path);
                  }}
                  onToggle={() => toggleFile(file.path)}
                  onRender={() => renderFile(file.path)}
                  onOpenFileTab={() => onOpenFileTab(file.path)}
                  onRevert={() => onRevertFile(file.path)}
                  onExpandGap={(gapId) => expandGap(file.path, gapId)}
                  onRequestContent={() => requestContent(file.path)}
                  onShowMoreRows={() =>
                    setRowLimits((current) => ({
                      ...current,
                      [file.path]:
                        (current[file.path] ?? defaultRowLimit) + defaultRowLimit,
                    }))
                  }
                />
              ))}
            </>
          )}
        </div>

        {preferences.showFilePanel ? (
          <aside className="diff-review__files" aria-label="变更文件">
            <label className="diff-review__filter">
              <Search size={13} aria-hidden="true" />
              <input
                value={treeFilter}
                placeholder="筛选文件"
                aria-label="筛选文件"
                onChange={(event) => setTreeFilter(event.target.value)}
              />
            </label>
            <div
              className="diff-review__tree"
              role="tree"
              aria-label="变更文件树"
            >
              {filteredTree.length ? (
                filteredTree.map((node) => (
                  <DiffTreeRow
                    key={node.id}
                    node={node}
                    depth={0}
                    activePath={activePath}
                    onSelect={scrollToFile}
                    onOpenFileTab={onOpenFileTab}
                  />
                ))
              ) : (
                <p className="diff-review__empty compact">没有匹配的文件。</p>
              )}
            </div>
          </aside>
        ) : null}
      </div>
    </div>
  );
}

/* ----------------------------------------------------------------- toolbar */

function ScopePicker({
  scopes,
  activeId,
  onSelect,
}: {
  scopes: ReviewScope[];
  activeId: string;
  onSelect(id: string): void;
}) {
  const active = scopes.find((scope) => scope.id === activeId) ?? scopes[0];
  return (
    <div className="diff-review__menu">
      <Popover
        label="选择审阅范围"
        align="start"
        trigger={(props) => (
          <button className="diff-review__scope" type="button" {...props}>
            <span>{active?.label ?? "审阅范围"}</span>
            <ChevronDown size={14} aria-hidden="true" />
          </button>
        )}
      >
        {({ close }) => (
          <div className="diff-review__menu-list" role="menu">
            {scopes.map((scope) => (
              <button
                key={scope.id}
                className="diff-review__menu-item"
                type="button"
                role="menuitemradio"
                aria-checked={scope.id === activeId}
                onClick={() => {
                  onSelect(scope.id);
                  close();
                }}
              >
                <span className="diff-review__menu-check" aria-hidden="true">
                  {scope.id === activeId ? <Check size={13} /> : null}
                </span>
                <span>{scope.label}</span>
                {scope.kind === "turn" ? (
                  <span className="diff-review__menu-meta">
                    <span className="is-addition">+{scope.turn.additions}</span>
                    <span className="is-deletion">-{scope.turn.deletions}</span>
                  </span>
                ) : null}
              </button>
            ))}
          </div>
        )}
      </Popover>
    </div>
  );
}

function OptionsMenu({
  preferences,
  isRefreshing,
  canCopyPatch,
  onUpdate,
  onRefresh,
  onCopyGitApply,
}: {
  preferences: DiffReviewPreferences;
  isRefreshing: boolean;
  canCopyPatch: boolean;
  onUpdate(patch: Partial<DiffReviewPreferences>): void;
  onRefresh(): void;
  onCopyGitApply(): void;
}) {
  const toggles: Array<{
    key: keyof DiffReviewPreferences;
    label: string;
    hint: string;
    checked: boolean;
  }> = [
    {
      key: "wrapLines",
      label: "自动换行",
      hint: "长行折行显示，不再横向滚动",
      checked: preferences.wrapLines,
    },
    {
      key: "loadFullFile",
      label: "加载完整文件",
      hint: "读取工作区文件，展开所有未修改区域",
      checked: preferences.loadFullFile,
    },
    {
      key: "richPreview",
      label: "富文本预览",
      hint: "Markdown 按渲染结果显示",
      checked: preferences.richPreview,
    },
    {
      key: "wordDiff",
      label: "文字差异",
      hint: "在行内高亮改动的词",
      checked: preferences.wordDiff,
    },
    {
      key: "hideWhitespace",
      label: "隐藏空白字符",
      hint: "只有空白变化的行视为未修改",
      checked: preferences.hideWhitespace,
    },
  ];

  return (
    <div className="diff-review__menu">
      <Popover
        label="差异显示选项"
        align="end"
        trigger={(props) => (
          <IconButton aria-label="差异显示选项" size="compact" {...props}>
            <MoreHorizontal size={14} />
          </IconButton>
        )}
      >
        {({ close }) => (
          <div className="diff-review__menu-list" role="menu">
            <button
              className="diff-review__menu-item"
              type="button"
              role="menuitem"
              disabled={isRefreshing}
              onClick={() => {
                onRefresh();
                close();
              }}
            >
              <span className="diff-review__menu-check" aria-hidden="true">
                <RefreshCw className={isRefreshing ? "spin" : ""} size={13} />
              </span>
              <span>刷新</span>
            </button>
            {toggles.map((toggle) => (
              <button
                key={toggle.key}
                className="diff-review__menu-item"
                type="button"
                role="menuitemcheckbox"
                aria-checked={toggle.checked}
                title={toggle.hint}
                onClick={() => onUpdate({ [toggle.key]: !toggle.checked })}
              >
                <span className="diff-review__menu-check" aria-hidden="true">
                  {toggle.checked ? <Check size={13} /> : null}
                </span>
                <span>{toggle.label}</span>
              </button>
            ))}
            <button
              className="diff-review__menu-item"
              type="button"
              role="menuitem"
              disabled={!canCopyPatch}
              title={
                canCopyPatch
                  ? "复制可直接粘贴运行的补丁命令"
                  : "当前范围没有可复制的补丁"
              }
              onClick={() => {
                onCopyGitApply();
                close();
              }}
            >
              <span className="diff-review__menu-check" aria-hidden="true">
                <Copy size={13} />
              </span>
              <span>复制 git apply 命令</span>
            </button>
          </div>
        )}
      </Popover>
    </div>
  );
}

function JumpToFilePicker({
  files,
  onSelect,
}: {
  files: ParsedDiffFile[];
  onSelect(path: string): void;
}) {
  const [query, setQuery] = useState("");
  const matches = files.filter((file) => matchesPathQuery(file.path, query));

  return (
    <div className="diff-review__menu">
      <Popover
        label="跳转到文件"
        align="end"
        trigger={(props) => (
          <IconButton
            aria-label="跳转到文件"
            title="跳转到文件"
            size="compact"
            disabled={!files.length}
            {...props}
          >
            <FileCode2 size={14} />
          </IconButton>
        )}
      >
        {({ close }) => (
          <div className="diff-review__jump">
            <label className="diff-review__filter">
              <Search size={13} aria-hidden="true" />
              <input
                autoFocus
                value={query}
                placeholder="跳转到文件"
                aria-label="跳转到文件"
                onChange={(event) => setQuery(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key !== "Enter" || !matches[0]) return;
                  onSelect(matches[0].path);
                  close();
                }}
              />
            </label>
            <div className="diff-review__jump-list" role="listbox">
              {matches.length ? (
                matches.map((file) => (
                  <button
                    key={file.path}
                    className="diff-review__jump-item"
                    type="button"
                    role="option"
                    aria-selected={false}
                    onClick={() => {
                      onSelect(file.path);
                      close();
                    }}
                  >
                    <strong>{diffFileName(file.path)}</strong>
                    <span>{diffFileDirectory(file.path)}</span>
                  </button>
                ))
              ) : (
                <p className="diff-review__empty compact">没有匹配的文件。</p>
              )}
            </div>
          </div>
        )}
      </Popover>
    </div>
  );
}

function CommitMenu({
  canRunGit,
  changedFiles,
  onGitAction,
}: {
  canRunGit: boolean;
  changedFiles: number;
  onGitAction(action: DiffReviewGitAction, message: string): Promise<string>;
}) {
  const messageId = useId();
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState<DiffReviewGitAction | null>(null);
  const [result, setResult] = useState<{
    kind: "ok" | "error";
    text: string;
  } | null>(null);

  const run = (action: DiffReviewGitAction) => {
    if (busy) return;
    setBusy(action);
    setResult(null);
    onGitAction(action, message.trim())
      .then((text) => {
        setResult({ kind: "ok", text });
        if (action !== "push") setMessage("");
      })
      .catch((error: unknown) =>
        setResult({ kind: "error", text: errorMessage(error) }),
      )
      .finally(() => setBusy(null));
  };

  const needsMessage = !message.trim();
  const hasChanges = changedFiles > 0;

  return (
    <div className="diff-review__menu">
      <Popover
        label="提交或推送"
        align="end"
        trigger={(props) => (
          <button
            className="diff-review__commit"
            type="button"
            disabled={!canRunGit}
            title={canRunGit ? "提交或推送" : "当前工作区不是 Git 仓库"}
            {...props}
          >
            <GitCommitHorizontal size={14} aria-hidden="true" />
            <span>提交或推送</span>
            <ChevronDown size={14} aria-hidden="true" />
          </button>
        )}
      >
        {() => (
          <div className="diff-review__commit-form">
            <label className="diff-review__commit-label" htmlFor={messageId}>
              提交信息
            </label>
            <textarea
              id={messageId}
              className="diff-review__commit-input"
              rows={3}
              value={message}
              placeholder={`提交 ${changedFiles} 个文件的改动`}
              onChange={(event) => setMessage(event.target.value)}
            />
            <div className="diff-review__commit-actions">
              <Button
                size="compact"
                disabled={!hasChanges || needsMessage || busy !== null}
                onClick={() => run("commit")}
              >
                {busy === "commit" ? "提交中" : "提交"}
              </Button>
              <Button
                size="compact"
                variant="primary"
                disabled={!hasChanges || needsMessage || busy !== null}
                onClick={() => run("commit_push")}
              >
                {busy === "commit_push" ? "处理中" : "提交并推送"}
              </Button>
              <Button
                size="compact"
                variant="quiet"
                disabled={busy !== null}
                onClick={() => run("push")}
              >
                {busy === "push" ? "推送中" : "仅推送"}
              </Button>
            </div>
            {result ? (
              <p
                className={`diff-review__commit-result ${result.kind === "error" ? "is-error" : ""}`}
                role={result.kind === "error" ? "alert" : "status"}
              >
                {result.text}
              </p>
            ) : null}
          </div>
        )}
      </Popover>
    </div>
  );
}

/* -------------------------------------------------------------------- file */

type DiffFileSectionProps = {
  file: ParsedDiffFile;
  content: ContentState | null;
  collapsed: boolean;
  active: boolean;
  preferences: DiffReviewPreferences;
  buildOptions: DiffBuildOptions;
  expandedGaps: ReadonlySet<string>;
  rowLimit: number;
  isReverting: boolean;
  revertBlockedReason: string | null;
  registerSection(element: HTMLElement | null): void;
  onToggle(): void;
  onOpenFileTab(): void;
  onRevert(): void;
  onExpandGap(gapId: string): void;
  onRequestContent(): void;
  onShowMoreRows(): void;
};

function DeferredDiffFileSection({
  renderBody,
  onRender,
  ...props
}: DiffFileSectionProps & { renderBody: boolean; onRender(): void }) {
  if (renderBody) return <DiffFileSection {...props} />;

  const {
    file,
    collapsed,
    active,
    isReverting,
    revertBlockedReason,
    registerSection,
    onToggle,
    onOpenFileTab,
    onRevert,
  } = props;
  const language = diffLanguageFromPath(file.path);
  const additions = fileAdditions(file);
  const deletions = fileDeletions(file);

  return (
    <section
      className="diff-review__file"
      data-file-path={file.path}
      data-active={active || undefined}
      ref={registerSection}
      aria-label={file.path}
    >
      <header className="diff-review__file-header">
        <button
          className="diff-review__file-toggle"
          type="button"
          aria-expanded={!collapsed}
          aria-label={collapsed ? `展开 ${file.path}` : `折叠 ${file.path}`}
          onClick={onToggle}
        >
          {collapsed ? <ChevronRight size={14} /> : <ChevronDown size={14} />}
        </button>
        <span className="diff-review__file-icon" aria-hidden="true">
          {language ? <FileCode2 size={14} /> : <FileText size={14} />}
        </span>
        <span className="diff-review__file-path" title={file.path}>
          {diffFileDirectory(file.path) ? (
            <span className="diff-review__file-dir">
              {diffFileDirectory(file.path)}/
            </span>
          ) : null}
          <strong>{diffFileName(file.path)}</strong>
        </span>
        <span className="diff-review__file-status">{statusLabel(file)}</span>
        <span
          className="diff-review__stats"
          aria-label={`增加 ${additions} 行，删除 ${deletions} 行`}
        >
          <span className="is-addition">+{additions}</span>
          <span className="is-deletion">-{deletions}</span>
        </span>
        <span className="diff-review__file-actions">
          <IconButton
            aria-label={`在标签页中打开 ${file.path}`}
            title="在标签页中打开文件"
            size="compact"
            onClick={onOpenFileTab}
          >
            <ExternalLink size={13} />
          </IconButton>
          <IconButton
            aria-label={`还原 ${file.path}`}
            title={revertBlockedReason ?? "还原此文件到 HEAD"}
            size="compact"
            disabled={Boolean(revertBlockedReason) || isReverting}
            onClick={onRevert}
          >
            <RotateCcw className={isReverting ? "spin" : ""} size={13} />
          </IconButton>
        </span>
      </header>
      {collapsed ? null : (
        <div className="diff-review__empty compact">
          <Button size="compact" variant="quiet" onClick={onRender}>
            显示差异
          </Button>
        </div>
      )}
    </section>
  );
}

function DiffFileSection({
  file,
  content,
  collapsed,
  active,
  preferences,
  buildOptions,
  expandedGaps,
  rowLimit,
  isReverting,
  revertBlockedReason,
  registerSection,
  onToggle,
  onOpenFileTab,
  onRevert,
  onExpandGap,
  onRequestContent,
  onShowMoreRows,
}: DiffFileSectionProps) {
  const language = useMemo(() => diffLanguageFromPath(file.path), [file.path]);
  // Truncated content would misnumber every expanded line, so it is only used
  // for the rich preview, never to fill in untouched regions.
  const contentLines =
    content?.status === "ready" && !content.truncated
      ? (content.lines ?? null)
      : null;

  const effectiveFile = useMemo(
    () =>
      file.hunks.length || !contentLines
        ? file
        : withLoadedContent(file, contentLines),
    [contentLines, file],
  );

  const options = useMemo<DiffBuildOptions>(
    () => ({
      ...buildOptions,
      language,
      newFileLines: contentLines,
      expandedGaps: preferences.loadFullFile
        ? "all"
        : localGapIds(expandedGaps, file.path),
    }),
    [
      buildOptions,
      contentLines,
      expandedGaps,
      file.path,
      language,
      preferences.loadFullFile,
    ],
  );

  const blocks = useMemo(
    () => buildDiffBlocks(effectiveFile, options),
    [effectiveFile, options],
  );
  const rows = useMemo<Array<DiffSplitRow | DiffUnifiedRow>>(
    () =>
      preferences.view === "split"
        ? buildSplitRows(blocks, options, rowLimit)
        : buildUnifiedRows(blocks, options, rowLimit),
    [blocks, options, preferences.view, rowLimit],
  );
  const totalRows = useMemo(
    () => countDiffRows(blocks, preferences.view),
    [blocks, preferences.view],
  );

  // Reads from the effective file so an untracked file counts its lines once
  // the content behind it has been loaded.
  const additions = fileAdditions(effectiveFile);
  const deletions = fileDeletions(effectiveFile);
  const hiddenRows = totalRows - rows.length;
  const showRichPreview =
    preferences.richPreview &&
    isRichPreviewPath(file.path) &&
    content?.status === "ready";

  return (
    <section
      className="diff-review__file"
      data-file-path={file.path}
      data-active={active || undefined}
      ref={registerSection}
      aria-label={file.path}
    >
      <header className="diff-review__file-header">
        <button
          className="diff-review__file-toggle"
          type="button"
          aria-expanded={!collapsed}
          aria-label={collapsed ? `展开 ${file.path}` : `折叠 ${file.path}`}
          onClick={onToggle}
        >
          {collapsed ? <ChevronRight size={14} /> : <ChevronDown size={14} />}
        </button>
        <span className="diff-review__file-icon" aria-hidden="true">
          {language ? <FileCode2 size={14} /> : <FileText size={14} />}
        </span>
        <span className="diff-review__file-path" title={file.path}>
          {diffFileDirectory(file.path) ? (
            <span className="diff-review__file-dir">
              {diffFileDirectory(file.path)}/
            </span>
          ) : null}
          <strong>{diffFileName(file.path)}</strong>
        </span>
        <span className="diff-review__file-status">{statusLabel(file)}</span>
        <span
          className="diff-review__stats"
          aria-label={`增加 ${additions} 行，删除 ${deletions} 行`}
        >
          <span className="is-addition">+{additions}</span>
          <span className="is-deletion">-{deletions}</span>
        </span>
        <span className="diff-review__file-actions">
          <IconButton
            aria-label={`在标签页中打开 ${file.path}`}
            title="在标签页中打开文件"
            size="compact"
            onClick={onOpenFileTab}
          >
            <ExternalLink size={13} />
          </IconButton>
          <IconButton
            aria-label={`还原 ${file.path}`}
            title={revertBlockedReason ?? "还原此文件到 HEAD"}
            size="compact"
            disabled={Boolean(revertBlockedReason) || isReverting}
            onClick={onRevert}
          >
            <RotateCcw className={isReverting ? "spin" : ""} size={13} />
          </IconButton>
        </span>
      </header>

      {collapsed ? null : file.binary ? (
        <p className="diff-review__empty compact">
          这是二进制文件，没有可显示的文本差异。
        </p>
      ) : showRichPreview ? (
        <div className="diff-review__rich">
          <MarkdownContent text={content?.text ?? ""} />
        </div>
      ) : rows.length === 0 ? (
        <FileBodyPlaceholder
          content={content}
          onRequestContent={onRequestContent}
        />
      ) : (
        <>
          <div
            className="diff-review__rows"
            role="table"
            aria-label={file.path}
          >
            <div className="diff-review__grid">
              {rows.map((row) =>
                row.type === "gap" ? (
                  <button
                    key={row.id}
                    className="diff-review__gap"
                    type="button"
                    onClick={() => onExpandGap(row.id)}
                  >
                    <UnfoldVertical size={13} aria-hidden="true" />
                    <span>
                      {row.count} 行未修改
                      {content?.status === "loading" ? "（加载中…）" : ""}
                      {content?.status === "error" ? "（无法读取文件）" : ""}
                    </span>
                  </button>
                ) : row.type === "pair" ? (
                  <div className="diff-review__row" role="row" key={row.id}>
                    <LineNumber side={row.left} />
                    <CodeCell side={row.left} />
                    <LineNumber side={row.right} />
                    <CodeCell side={row.right} />
                  </div>
                ) : (
                  <div className="diff-review__row" role="row" key={row.id}>
                    <span className="diff-review__gutter" role="cell">
                      {row.oldLine ?? ""}
                    </span>
                    <span className="diff-review__gutter" role="cell">
                      {row.newLine ?? ""}
                    </span>
                    <CodeCell side={row.side} />
                  </div>
                ),
              )}
            </div>
          </div>
          {hiddenRows > 0 ? (
            <button
              className="diff-review__more"
              type="button"
              onClick={onShowMoreRows}
            >
              还有 {hiddenRows} 行，继续显示
            </button>
          ) : null}
        </>
      )}
    </section>
  );
}

function FileBodyPlaceholder({
  content,
  onRequestContent,
}: {
  content: ContentState | null;
  onRequestContent(): void;
}) {
  if (content?.status === "loading") {
    return (
      <p className="diff-review__empty compact">
        <Loader2 className="spin" size={14} aria-hidden="true" />
        正在读取文件…
      </p>
    );
  }
  if (content?.status === "error") {
    return (
      <p className="diff-review__empty compact is-error">
        <AlertCircle size={14} aria-hidden="true" />
        {content.error}
      </p>
    );
  }
  return (
    <div className="diff-review__empty compact">
      <span>该文件没有文本差异（可能是新增或未跟踪文件）。</span>
      <Button size="compact" variant="quiet" onClick={onRequestContent}>
        读取文件内容
      </Button>
    </div>
  );
}

function LineNumber({ side }: { side: DiffRowSide | null }) {
  return (
    <span
      className="diff-review__gutter"
      role="cell"
      data-kind={side?.kind ?? "filler"}
    >
      {side?.number ?? ""}
    </span>
  );
}

function CodeCell({ side }: { side: DiffRowSide | null }) {
  if (!side) {
    return (
      <span
        className="diff-review__code diff-review__code--filler"
        role="cell"
        aria-hidden="true"
      />
    );
  }
  return (
    <span className="diff-review__code" role="cell" data-kind={side.kind}>
      <span className="diff-review__sign" aria-hidden="true">
        {side.kind === "added" ? "+" : side.kind === "removed" ? "-" : " "}
      </span>
      {side.spans.map((span, index) => (
        <span key={index} className={spanClassName(span)}>
          {span.text}
        </span>
      ))}
    </span>
  );
}

function spanClassName(span: DiffSpan): string {
  return [
    span.syntax ? `diff-review__token--${span.syntax}` : "",
    span.changed ? "is-changed" : "",
  ]
    .filter(Boolean)
    .join(" ");
}

/* ---------------------------------------------------------------- file tree */

function DiffTreeRow({
  node,
  depth,
  activePath,
  onSelect,
  onOpenFileTab,
}: {
  node: DiffTreeNode;
  depth: number;
  activePath: string | null;
  onSelect(path: string): void;
  onOpenFileTab(path: string): void;
}): ReactNode {
  const [collapsed, setCollapsed] = useState(false);

  if (node.type === "directory") {
    return (
      <div className="diff-review__tree-group" role="group">
        <button
          className="diff-review__tree-row"
          type="button"
          role="treeitem"
          aria-expanded={!collapsed}
          style={{
            paddingLeft: `calc(var(--space-4) + ${depth} * var(--space-6))`,
          }}
          onClick={() => setCollapsed((value) => !value)}
        >
          {collapsed ? <ChevronRight size={13} /> : <ChevronDown size={13} />}
          <Folder size={13} aria-hidden="true" />
          <span className="diff-review__tree-name">{node.name}</span>
        </button>
        {collapsed
          ? null
          : node.children.map((child) => (
              <DiffTreeRow
                key={child.id}
                node={child}
                depth={depth + 1}
                activePath={activePath}
                onSelect={onSelect}
                onOpenFileTab={onOpenFileTab}
              />
            ))}
      </div>
    );
  }

  return (
    <div className="diff-review__tree-file">
      <button
        className="diff-review__tree-row"
        type="button"
        role="treeitem"
        aria-selected={node.path === activePath}
        data-active={node.path === activePath || undefined}
        title={node.path}
        style={{
          paddingLeft: `calc(var(--space-6) + ${depth} * var(--space-6))`,
        }}
        onClick={() => onSelect(node.path)}
      >
        <span className="diff-review__tree-name">{node.name}</span>
        <span className="diff-review__stats">
          <span className="is-addition">+{node.additions}</span>
          <span className="is-deletion">-{node.deletions}</span>
        </span>
      </button>
      <IconButton
        aria-label={`在标签页中打开 ${node.path}`}
        title="在标签页中打开文件"
        size="compact"
        className="diff-review__tree-open"
        onClick={() => onOpenFileTab(node.path)}
      >
        <ExternalLink size={12} />
      </IconButton>
    </div>
  );
}

function filterTree(nodes: DiffTreeNode[], query: string): DiffTreeNode[] {
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

/* ----------------------------------------------------------------- helpers */

function buildWorkspaceFiles(diff: WorkspaceDiff | null): ParsedDiffFile[] {
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

function emptyPlaceholderFile(
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

function binaryPlaceholderFile(path: string): ParsedDiffFile {
  return { ...emptyPlaceholderFile(path), binary: true };
}

/** Turns a loaded file into an all-added hunk so untracked files can be read. */
function withLoadedContent(
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

function localGapIds(
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

function statusLabel(file: ParsedDiffFile): string {
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

function isRichPreviewPath(path: string): boolean {
  return /\.(md|markdown|mdx)$/i.test(path);
}

function normalizePath(path: string): string {
  return path.replace(/\\/g, "/");
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
