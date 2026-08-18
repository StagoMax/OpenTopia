import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  AlertCircle,
  Columns2,
  FoldVertical,
  Loader2,
  PanelRight,
  Rows3,
  Search,
  UnfoldVertical,
} from "lucide-react";
import {
  buildDiffFileTree,
  buildGitApplyCommand,
  parseUnifiedDiff,
  splitFileContent,
  summarizeDiffStats,
  type DiffBuildOptions,
  type ParsedDiffFile,
} from "../diffReview";
import {
  readDiffReviewPreferences,
  writeDiffReviewPreferences,
  type DiffReviewPreferences,
} from "../diffReviewPreferences";
import { DeferredDiffFileSection } from "./diffReviewPanel/DiffFileSection";
import { DiffTreeRow } from "./diffReviewPanel/FileTree";
import {
  BranchPicker,
  CommitMenu,
  JumpToFilePicker,
  OptionsMenu,
  ScopePicker,
} from "./diffReviewPanel/ToolbarControls";
import {
  binaryPlaceholderFile,
  buildWorkspaceFiles,
  defaultRowLimit,
  emptyPlaceholderFile,
  errorMessage,
  filterTree,
  initialRenderedFileCount,
  isRichPreviewPath,
  turnDiffConcurrency,
  workspaceScopeId,
  type ContentState,
  type DiffReviewPanelProps,
  type DiffReviewTurnScope,
  type ReviewScope,
  type TurnFilesState,
} from "./diffReviewPanel/model";
import { IconButton } from "./ui";
import "./DiffReviewPanel.css";

export type {
  DiffReviewFileContent,
  DiffReviewGitAction,
  DiffReviewPanelProps,
  DiffReviewTurnScope,
} from "./diffReviewPanel/model";

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
  onListGitBranches,
  onSwitchGitBranch,
}: DiffReviewPanelProps) {
  const [preferences, setPreferences] = useState<DiffReviewPreferences>(() => ({
    ...readDiffReviewPreferences(),
    view: "unified",
    wordDiff: false,
  }));
  const [scopeId, setScopeId] = useState<string>(workspaceScopeId);
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
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
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

  // The review entry point is always the current workspace, never a single
  // agent turn. Individual files are selected from that complete change set.
  const scopePickedRef = useRef(false);
  const selectScope = useCallback((id: string) => {
    scopePickedRef.current = true;
    setScopeId(id);
  }, []);

  useEffect(() => {
    setScopeId(workspaceScopeId);
  }, []);

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
  const selectedFile =
    files.find((file) => file.path === selectedPath) ?? files[0] ?? null;

  useEffect(() => {
    if (!files.length) {
      setSelectedPath(null);
      return;
    }
    if (!files.some((file) => file.path === selectedPath)) {
      setSelectedPath(files[0].path);
    }
  }, [files, selectedPath]);

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
        if (path)
          setActivePath((current) => (current === path ? current : path));
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

  const toggleFile = useCallback(
    (path: string) => {
      renderFile(path);
      setCollapsedFiles((current) => {
        const next = new Set(current);
        if (next.has(path)) next.delete(path);
        else next.add(path);
        return next;
      });
    },
    [renderFile],
  );

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

  const scrollToFile = useCallback(
    (path: string) => {
      setSelectedPath(path);
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
    },
    [renderFile],
  );

  // A "review this file" request names a working-tree path, so it always
  // resolves against the workspace baseline rather than a recorded turn.
  useEffect(() => {
    if (!focusRequest) return;
    selectScope(workspaceScopeId);
    setSelectedPath(focusRequest.path);
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
      wordDiff: false,
    }),
    [preferences.hideWhitespace],
  );

  return (
    <div
      className="diff-review"
      data-view={preferences.view}
      data-wrap={preferences.wrapLines ? "on" : "off"}
    >
      <header className="diff-review__toolbar">
        <div className="diff-review__toolbar-main">
          <div className="diff-review__toolbar-context">
            <span className="diff-review__all-changes">全部修改</span>
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
          </div>
          <div className="diff-review__toolbar-actions">
            {notice ? (
              <span className="diff-review__notice" role="status">
                {notice}
              </span>
            ) : null}
            <OptionsMenu
              preferences={preferences}
              isRefreshing={isRefreshing}
              canCopyPatch={false}
              onUpdate={update}
              onRefresh={onRefresh}
              onCopyGitApply={copyGitApply}
            />
            <IconButton
              aria-label={allCollapsed ? "展开全部差异" : "折叠全部差异"}
              title={allCollapsed ? "展开全部差异" : "折叠全部差异"}
              className="diff-review__legacy-control"
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
            <JumpToFilePicker
              files={files}
              selectedPath={selectedFile?.path ?? null}
              onSelect={scrollToFile}
            />
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
                update({
                  view: preferences.view === "split" ? "unified" : "split",
                })
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
              className="diff-review__file-panel-toggle"
              size="compact"
              aria-pressed={preferences.showFilePanel}
              onClick={() =>
                update({ showFilePanel: !preferences.showFilePanel })
              }
            >
              <PanelRight size={14} />
            </IconButton>
            <CommitMenu
              canRunGit={canRunGit}
              changedFiles={files.length}
              onGitAction={onGitAction}
            />
          </div>
        </div>
        <div className="diff-review__branch-row">
          <BranchPicker
            currentBranch={workspaceDiff?.branch?.trim() || "HEAD"}
            disabled={!canRunGit}
            onList={onListGitBranches}
            onSwitch={onSwitchGitBranch}
          />
        </div>
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
                  正在加载 {turnState.loadedFileCount}/
                  {turnState.totalFileCount} 个文件…
                </p>
              ) : null}
              {turnState?.error ? (
                <p
                  className="diff-review__empty compact is-error"
                  role="status"
                >
                  <AlertCircle size={14} aria-hidden="true" />
                  {turnState.error}
                </p>
              ) : null}
              {files
                .filter((file) => file.path === selectedFile?.path)
                .map((file) => (
                  <DeferredDiffFileSection
                    key={file.path}
                    file={file}
                    content={contents[file.path] ?? null}
                    collapsed={false}
                    active
                    renderBody
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
                    onToggle={() => undefined}
                    onRender={() => renderFile(file.path)}
                    onOpenFileTab={() => onOpenFileTab(file.path)}
                    onRevert={() => onRevertFile(file.path)}
                    onExpandGap={(gapId) => expandGap(file.path, gapId)}
                    onRequestContent={() => requestContent(file.path)}
                    onShowMoreRows={() =>
                      setRowLimits((current) => ({
                        ...current,
                        [file.path]:
                          (current[file.path] ?? defaultRowLimit) +
                          defaultRowLimit,
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
