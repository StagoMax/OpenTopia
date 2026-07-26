import {
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  Activity,
  AlertCircle,
  Bot,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  ChevronUp,
  Clock3,
  Code2,
  FileDiff,
  FileSearch,
  FileText,
  Globe2,
  Loader2,
  RotateCcw,
  Table2,
  TerminalSquare,
  Wrench,
  X,
} from "lucide-react";
import type {
  AgentEvent,
  SubagentRun,
  TaskPlan,
  ToolCall,
  ToolResult,
  TurnChangeSet,
  TurnFileDiffPreview,
  TurnFileChange,
} from "../types";
import { MarkdownContent } from "./MarkdownContent";
import "./TurnActivityTimeline.css";

type ToolCategory =
  "command" | "file" | "browser" | "spreadsheet" | "agent" | "tool";

type ToolExecution = {
  call: ToolCall;
  startedAt: string;
  result?: ToolResult;
  finishedAt?: string;
};

type PlanStepTiming = {
  startedAt?: string;
  finishedAt?: string;
};

type FileChangeSummary = {
  path: string;
  operation: string;
  additions?: number;
  deletions?: number;
  detail?: string;
};

type ActivityFile = {
  path: string;
  summary: string;
  createdAt: string;
};

type PrimitiveActivity =
  | { kind: "tool"; seq: number; execution: ToolExecution }
  | {
      kind: "plan";
      seq: number;
      plan: TaskPlan;
      startedAt: string;
      finishedAt?: string;
      stepTimings: PlanStepTiming[];
    }
  | {
      kind: "file";
      seq: number;
      path: string;
      summary: string;
      createdAt: string;
    }
  | {
      kind: "reasoning";
      seq: number;
      text: string;
      isDelta: boolean;
      createdAt: string;
    }
  | {
      kind: "commentary";
      seq: number;
      text: string;
      createdAt: string;
    }
  | { kind: "subagent"; seq: number; run: SubagentRun; createdAt: string }
  | {
      kind: "approval";
      seq: number;
      reason: string;
      action: string;
      createdAt: string;
    }
  | { kind: "error"; seq: number; message: string; createdAt: string }
  | { kind: "cancelled"; seq: number; reason: string; createdAt: string }
  | { kind: "suspended"; seq: number; reason: string; createdAt: string };

type ActivityEntry =
  | {
      kind: "tool-group";
      id: string;
      category: ToolCategory;
      executions: ToolExecution[];
    }
  | {
      kind: "file-group";
      id: string;
      files: ActivityFile[];
    }
  | Exclude<PrimitiveActivity, { kind: "tool" } | { kind: "file" }>;

type ActivityState = "running" | "complete" | "waiting" | "cancelled" | "error";

const defaultVisibleTurnFiles = 3;
const defaultVisibleDiffLines = 48;

type TurnFilePreviewState = {
  key: string;
  loading: boolean;
  loadingMore: boolean;
  error: string | null;
  preview: TurnFileDiffPreview | null;
  visibleLines: number;
};

type TurnDiffLine = {
  kind: "context" | "added" | "deleted" | "hunk" | "meta";
  text: string;
  oldLine: number | null;
  newLine: number | null;
};

export function TurnActivityTimeline({
  events,
  isActive,
  formatError = (message) => message,
  onOpenMarkdownLink,
}: {
  events: AgentEvent[];
  isActive: boolean;
  formatError?(message: string): string;
  onOpenMarkdownLink?(href: string): void;
}) {
  const entries = useMemo(() => buildActivityEntries(events), [events]);
  const state = activityState(events, isActive);
  const [expanded, setExpanded] = useState(isActive);
  const bodyId = useId();
  const mountedAt = useMemo(() => Date.now(), []);
  const hasRunningEntry = entries.some(activityEntryIsRunning);
  const now = useTimelineClock(
    isActive || (state === "running" && hasRunningEntry),
  );
  const turnTiming = formatTurnTiming(events, isActive, now, mountedAt);

  useEffect(() => {
    if (isActive || state === "error" || state === "waiting") {
      setExpanded(true);
    } else {
      setExpanded(false);
    }
  }, [isActive, state]);

  const changeSetEvent = [...events]
    .reverse()
    .find((event) => event.payload.type === "turn_changes_recorded");
  const changeSet =
    changeSetEvent?.payload.type === "turn_changes_recorded"
      ? changeSetEvent.payload.change_set
      : null;
  const hasTurnLifecycle = events.some((event) =>
    [
      "turn_started",
      "turn_finished",
      "turn_cancelled",
      "turn_suspended",
      "error",
    ].includes(event.payload.type),
  );
  if (!isActive && entries.length === 0 && !changeSet && !hasTurnLifecycle) {
    return null;
  }

  return (
    <section className="turn-activity" data-state={state}>
      <button
        className="turn-activity-header"
        type="button"
        aria-expanded={expanded}
        aria-controls={bodyId}
        onClick={() => setExpanded((current) => !current)}
      >
        <span className="turn-activity-heading">
          <strong>{activityStateLabel(state, isActive)}</strong>
          {turnTiming && <small>{turnTiming}</small>}
        </span>
        <span className="turn-activity-chevron" aria-hidden="true">
          {expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
        </span>
      </button>

      {expanded && (
        <div
          className="turn-activity-body"
          id={bodyId}
          aria-live={isActive ? "polite" : undefined}
        >
          {entries.length === 0 && isActive && (
            <div className="turn-activity-pending" role="status">
              <ActivityFlow />
              <span>正在连接模型并准备执行步骤</span>
            </div>
          )}
          {entries.map((entry) => (
            <ActivityEntryView
              key={activityEntryKey(entry)}
              entry={entry}
              isActive={isActive}
              now={now}
              formatError={formatError}
              onOpenMarkdownLink={onOpenMarkdownLink}
            />
          ))}
          {changeSet?.status === "failed" && changeSet.error && (
            <div className="turn-change-set-warning" role="status">
              <AlertCircle size={13} />
              <span>未能记录本轮文件快照：{changeSet.error}</span>
            </div>
          )}
        </div>
      )}
    </section>
  );
}

export function TurnChangeCard({
  changeSet,
  isWorkspaceBusy = false,
  isUndoing = false,
  isReverted = false,
  onUndo,
  onReview,
  onLoadFilePreview,
}: {
  changeSet: TurnChangeSet;
  isWorkspaceBusy?: boolean;
  isUndoing?: boolean;
  isReverted?: boolean;
  onUndo?(): void;
  onReview?(): void;
  onLoadFilePreview?(
    path: string,
    offset?: number,
  ): Promise<TurnFileDiffPreview>;
}) {
  const [showAll, setShowAll] = useState(false);
  const [filePreviewState, setFilePreviewState] =
    useState<TurnFilePreviewState | null>(null);

  useEffect(() => {
    setShowAll(false);
    setFilePreviewState(null);
  }, [changeSet.finalizedAt, changeSet.turnId]);

  if (changeSet.status !== "ready" || changeSet.files.length === 0) {
    return null;
  }

  const remaining = Math.max(
    0,
    changeSet.files.length - defaultVisibleTurnFiles,
  );
  const visibleFiles = showAll
    ? changeSet.files
    : changeSet.files.slice(0, defaultVisibleTurnFiles);
  const undoTitle = isWorkspaceBusy
    ? "轮次完成后可撤销"
    : isReverted
      ? "本轮修改已撤销"
      : "预览并撤销本轮文件修改";

  const loadFilePreview = (
    file: TurnFileChange,
    offset = 0,
    loadingMore = false,
  ) => {
    const key = turnChangeFileKey(file);
    const path = turnChangeFileRequestPath(file);
    if (!onLoadFilePreview || !path || file.binary) return;
    setFilePreviewState((current) => ({
      key,
      loading: !loadingMore,
      loadingMore,
      error: null,
      preview: loadingMore ? (current?.preview ?? null) : null,
      visibleLines: loadingMore
        ? (current?.visibleLines ?? defaultVisibleDiffLines)
        : defaultVisibleDiffLines,
    }));
    void onLoadFilePreview(path, offset)
      .then((preview) => {
        setFilePreviewState((current) => {
          if (current?.key !== key) return current;
          const combinedPreview =
            loadingMore && current.preview
              ? {
                  ...preview,
                  diff: current.preview.diff + preview.diff,
                  offset: 0,
                }
              : preview;
          return {
            key,
            loading: false,
            loadingMore: false,
            error: null,
            preview: combinedPreview,
            visibleLines: loadingMore
              ? current.visibleLines + defaultVisibleDiffLines
              : defaultVisibleDiffLines,
          };
        });
      })
      .catch((error: unknown) => {
        setFilePreviewState((current) =>
          current?.key === key
            ? {
                ...current,
                loading: false,
                loadingMore: false,
                error: turnFilePreviewError(error),
              }
            : current,
        );
      });
  };

  const openFilePreview = (file: TurnFileChange) => {
    const key = turnChangeFileKey(file);
    if (filePreviewState?.key === key) return;
    if (file.binary) {
      setFilePreviewState({
        key,
        loading: false,
        loadingMore: false,
        error: null,
        preview: null,
        visibleLines: defaultVisibleDiffLines,
      });
      return;
    }
    loadFilePreview(file);
  };

  const closeFilePreview = (file: TurnFileChange) => {
    const key = turnChangeFileKey(file);
    setFilePreviewState((current) => (current?.key === key ? null : current));
  };

  return (
    <article
      className="turn-change-card"
      data-reverted={isReverted || undefined}
      aria-label={`本轮修改了 ${changeSet.files.length} 个文件`}
    >
      <header className="turn-change-card-header">
        <span className="turn-change-card-icon" aria-hidden="true">
          <FileDiff size={18} />
        </span>
        <div className="turn-change-card-title">
          <strong>
            {isReverted ? "已撤销" : "已修改"} {changeSet.files.length} 个文件
          </strong>
          <span
            aria-label={`增加 ${changeSet.additions} 行，删除 ${changeSet.deletions} 行`}
          >
            <span className="file-change-additions">
              +{changeSet.additions}
            </span>{" "}
            <span className="file-change-deletions">
              -{changeSet.deletions}
            </span>
          </span>
        </div>
        <div className="turn-change-card-actions">
          {onUndo && (
            <button
              className="turn-change-card-action undo"
              type="button"
              disabled={isWorkspaceBusy || isUndoing || isReverted}
              aria-label={isReverted ? "本轮修改已撤销" : "撤销本轮文件修改"}
              title={undoTitle}
              onClick={onUndo}
            >
              {isUndoing ? (
                <Loader2 className="spin" size={14} aria-hidden="true" />
              ) : (
                <RotateCcw size={14} aria-hidden="true" />
              )}
              <span>
                {isReverted ? "已撤销" : isUndoing ? "检查中" : "撤销"}
              </span>
            </button>
          )}
          {onReview && (
            <button
              className="turn-change-card-action review"
              type="button"
              disabled={isWorkspaceBusy || isReverted}
              aria-label="打开差异审核"
              title={
                isWorkspaceBusy
                  ? "轮次完成后可审核"
                  : isReverted
                    ? "本轮修改已撤销"
                    : "在差异面板中审核当前工作区"
              }
              onClick={onReview}
            >
              <FileSearch size={14} aria-hidden="true" />
              <span>审核</span>
            </button>
          )}
        </div>
      </header>

      <div
        className="turn-change-card-files"
        role="list"
        aria-label="本轮文件变更"
      >
        {visibleFiles.map((file, index) => (
          <TurnChangeFileRow
            key={turnChangeFileKey(file)}
            file={file}
            previewId={`turn-file-preview-${changeSet.turnId}-${index}`}
            previewState={
              filePreviewState?.key === turnChangeFileKey(file)
                ? filePreviewState
                : null
            }
            canPreview={file.binary || Boolean(onLoadFilePreview)}
            onOpen={() => openFilePreview(file)}
            onClose={() => closeFilePreview(file)}
            onRetry={() => loadFilePreview(file)}
            onShowMoreLines={() =>
              setFilePreviewState((current) =>
                current?.key === turnChangeFileKey(file)
                  ? {
                      ...current,
                      visibleLines:
                        current.visibleLines + defaultVisibleDiffLines,
                    }
                  : current,
              )
            }
            onLoadMore={() => {
              const nextOffset = filePreviewState?.preview?.nextOffset;
              if (nextOffset !== null && nextOffset !== undefined) {
                loadFilePreview(file, nextOffset, true);
              }
            }}
          />
        ))}
      </div>

      {remaining > 0 && (
        <button
          className="turn-change-card-more"
          type="button"
          aria-expanded={showAll}
          onClick={() => {
            setShowAll((current) => !current);
            setFilePreviewState(null);
          }}
        >
          <span>{showAll ? "收起文件列表" : `再显示 ${remaining} 个文件`}</span>
          {showAll ? (
            <ChevronUp size={15} aria-hidden="true" />
          ) : (
            <ChevronDown size={15} aria-hidden="true" />
          )}
        </button>
      )}
    </article>
  );
}

function TurnChangeFileRow({
  file,
  previewId,
  previewState,
  canPreview,
  onOpen,
  onClose,
  onRetry,
  onShowMoreLines,
  onLoadMore,
}: {
  file: TurnFileChange;
  previewId: string;
  previewState: TurnFilePreviewState | null;
  canPreview: boolean;
  onOpen(): void;
  onClose(): void;
  onRetry(): void;
  onShowMoreLines(): void;
  onLoadMore(): void;
}) {
  const path = turnChangeFilePath(file);
  const kind = turnChangeKind(file.kind);
  const additions = file.additions ?? 0;
  const deletions = file.deletions ?? 0;
  const [pinned, setPinned] = useState(false);
  // A hover-triggered fetch renders nothing until it resolves (see
  // TurnChangeFilePreview), so the row must not advertise a panel that is not
  // on screen yet.
  const previewPending = Boolean(
    previewState &&
    !file.binary &&
    previewState.loading &&
    !previewState.preview &&
    !pinned,
  );
  const expanded = Boolean(previewState) && !previewPending;

  useEffect(() => {
    if (!previewState) setPinned(false);
  }, [previewState]);

  const closePreview = () => {
    if (!pinned) onClose();
  };

  return (
    <div
      className="turn-change-card-file"
      data-expanded={expanded || undefined}
      data-preview-visible={expanded || undefined}
      role="listitem"
      onMouseEnter={canPreview ? onOpen : undefined}
      onMouseLeave={canPreview ? closePreview : undefined}
      onFocus={canPreview ? onOpen : undefined}
      onBlur={
        canPreview
          ? (event) => {
              const nextTarget = event.relatedTarget;
              if (
                !(nextTarget instanceof Node) ||
                !event.currentTarget.contains(nextTarget)
              ) {
                closePreview();
              }
            }
          : undefined
      }
    >
      <button
        className="turn-change-card-file-button"
        type="button"
        disabled={!canPreview}
        aria-expanded={expanded}
        aria-haspopup="dialog"
        aria-controls={canPreview ? previewId : undefined}
        aria-label={`${expanded ? "收起" : "预览"} ${path} 的代码差异`}
        onKeyDown={(event) => {
          if (event.key !== "Escape" || !expanded) return;
          event.preventDefault();
          setPinned(false);
          onClose();
        }}
        onClick={() => {
          if (pinned) {
            setPinned(false);
            onClose();
            return;
          }
          setPinned(true);
          onOpen();
        }}
      >
        <span
          className="turn-change-card-file-kind"
          data-kind={file.kind}
          title={kind.label}
          aria-label={kind.label}
        >
          {kind.code}
        </span>
        <span className="turn-change-card-file-path" title={path}>
          {path}
        </span>
        {file.binary ? (
          <span className="turn-change-card-file-binary">二进制</span>
        ) : (
          <span
            className="turn-change-card-file-stats"
            aria-label={`增加 ${additions} 行，删除 ${deletions} 行`}
          >
            <span className="file-change-additions">+{additions}</span>
            <span className="file-change-deletions">-{deletions}</span>
          </span>
        )}
        <ChevronRight
          className="turn-change-card-file-chevron"
          size={15}
          aria-hidden="true"
        />
      </button>
      {previewState && (
        <TurnChangeFilePreview
          id={previewId}
          file={file}
          state={previewState}
          showLoadingState={pinned}
          onRetry={onRetry}
          onShowMoreLines={onShowMoreLines}
          onLoadMore={onLoadMore}
        />
      )}
    </div>
  );
}

function TurnChangeFilePreview({
  id,
  file,
  state,
  showLoadingState,
  onRetry,
  onShowMoreLines,
  onLoadMore,
}: {
  id: string;
  file: TurnFileChange;
  state: TurnFilePreviewState;
  showLoadingState: boolean;
  onRetry(): void;
  onShowMoreLines(): void;
  onLoadMore(): void;
}) {
  if (file.binary) {
    return (
      <div
        className="turn-change-card-preview-message"
        id={id}
        role="dialog"
        aria-label={`${turnChangeFilePath(file)} 的代码差异预览`}
      >
        <Code2 size={15} aria-hidden="true" />
        <span>这是二进制文件，没有可显示的文本代码差异。</span>
      </div>
    );
  }

  if (state.loading && !state.preview) {
    // Hovering a file row kicks off a fetch. Flashing a loading placeholder for
    // a pointer that is just passing through is pure noise, so the placeholder
    // is reserved for previews the user explicitly opened by clicking.
    if (!showLoadingState) return null;
    return (
      <div
        className="turn-change-card-preview-message"
        id={id}
        role="dialog"
        aria-busy="true"
        aria-label={`${turnChangeFilePath(file)} 的代码差异预览`}
      >
        <Loader2 className="spin" size={15} aria-hidden="true" />
        <span>正在加载历史代码差异...</span>
      </div>
    );
  }

  if (state.error) {
    return (
      <div
        className="turn-change-card-preview-message error"
        id={id}
        role="dialog"
        aria-label={`${turnChangeFilePath(file)} 的代码差异预览`}
      >
        <AlertCircle size={15} aria-hidden="true" />
        <span>{state.error}</span>
        <button type="button" onClick={onRetry}>
          重试
        </button>
      </div>
    );
  }

  const preview = state.preview;
  if (!preview) return null;
  const lines = parseTurnDiffLines(preview.diff);
  const visibleLines = lines.slice(0, state.visibleLines);
  const hiddenLines = Math.max(0, lines.length - visibleLines.length);
  const canLoadNextPage =
    preview.nextOffset !== null && preview.nextOffset !== undefined;
  const path = turnChangeFilePath(file);
  const loadedBytes = utf8ByteLength(preview.diff);
  // A pure addition has no old line numbers (and a pure deletion no new ones),
  // which would otherwise render as a permanently blank gutter column.
  const hasOldNumbers = lines.some((line) => line.oldLine !== null);
  const hasNewNumbers = lines.some((line) => line.newLine !== null);
  const numberColumns = (hasOldNumbers ? 1 : 0) + (hasNewNumbers ? 1 : 0);

  return (
    <section
      className="turn-change-card-preview"
      id={id}
      role="dialog"
      aria-label={`${path} 的代码差异预览`}
    >
      <header className="turn-change-card-preview-header">
        <code title={path}>{path}</code>
        <span className="turn-change-card-preview-meta">
          <span className="turn-change-card-file-stats">
            <span className="file-change-additions">
              +{file.additions ?? 0}
            </span>
            <span className="file-change-deletions">
              -{file.deletions ?? 0}
            </span>
          </span>
          <span>
            {canLoadNextPage
              ? `已加载 ${formatPreviewBytes(loadedBytes)} / ${formatPreviewBytes(preview.totalBytes)}`
              : `共 ${formatPreviewBytes(preview.totalBytes)}`}
          </span>
        </span>
      </header>
      {visibleLines.length > 0 ? (
        <div
          className="turn-change-card-preview-code"
          data-number-columns={numberColumns}
          role="table"
          tabIndex={0}
          aria-label="统一差异代码"
        >
          {visibleLines.map((line, index) => (
            <div
              className="turn-change-card-preview-line"
              data-kind={line.kind}
              role="row"
              key={`${index}:${line.oldLine ?? ""}:${line.newLine ?? ""}`}
            >
              {hasOldNumbers && (
                <span className="turn-change-card-preview-number" role="cell">
                  {line.oldLine ?? ""}
                </span>
              )}
              {hasNewNumbers && (
                <span className="turn-change-card-preview-number" role="cell">
                  {line.newLine ?? ""}
                </span>
              )}
              <code role="cell">{line.text || " "}</code>
            </div>
          ))}
        </div>
      ) : (
        <div className="turn-change-card-preview-message compact" role="status">
          该文件只有模式或元数据变化，没有文本代码差异。
        </div>
      )}
      {(hiddenLines > 0 || canLoadNextPage) && (
        <footer className="turn-change-card-preview-footer">
          <span>
            已显示 {visibleLines.length} 行
            {hiddenLines > 0 ? `，还有 ${hiddenLines} 行已加载` : ""}
          </span>
          <button
            type="button"
            disabled={state.loadingMore}
            onClick={hiddenLines > 0 ? onShowMoreLines : onLoadMore}
          >
            {state.loadingMore && (
              <Loader2 className="spin" size={13} aria-hidden="true" />
            )}
            {state.loadingMore
              ? "加载中"
              : hiddenLines > 0
                ? `再显示 ${Math.min(defaultVisibleDiffLines, hiddenLines)} 行`
                : "加载后续差异"}
          </button>
        </footer>
      )}
    </section>
  );
}

function parseTurnDiffLines(diff: string): TurnDiffLine[] {
  const sourceLines = diff.replace(/\r\n/g, "\n").split("\n");
  if (sourceLines.at(-1) === "") sourceLines.pop();
  let oldLine = 0;
  let newLine = 0;
  // Everything before the first hunk header is git plumbing ("diff --git",
  // "index ...", "--- /dev/null", "+++ b/..."). It is noise for the reader, so
  // it is dropped instead of rendered. Tracking the first hunk also keeps
  // content lines that happen to start with "---"/"+++" classified correctly.
  let inHunk = false;
  const lines: TurnDiffLine[] = [];

  for (const text of sourceLines) {
    // Re-arm the header skip at each file boundary so a multi-file diff does
    // not leak the second file's plumbing back into the output.
    if (text.startsWith("diff --git ")) {
      inHunk = false;
      continue;
    }
    const hunk = text.match(/^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/);
    if (hunk) {
      oldLine = Number(hunk[1]);
      newLine = Number(hunk[2]);
      inHunk = true;
      lines.push({ kind: "hunk", text, oldLine: null, newLine: null });
      continue;
    }
    if (!inHunk) continue;
    if (text.startsWith("+")) {
      const line = newLine;
      newLine += 1;
      lines.push({ kind: "added", text, oldLine: null, newLine: line });
      continue;
    }
    if (text.startsWith("-")) {
      const line = oldLine;
      oldLine += 1;
      lines.push({ kind: "deleted", text, oldLine: line, newLine: null });
      continue;
    }
    if (text.startsWith(" ")) {
      lines.push({ kind: "context", text, oldLine, newLine });
      oldLine += 1;
      newLine += 1;
      continue;
    }
    // "\ No newline at end of file" and friends.
    lines.push({ kind: "meta", text, oldLine: null, newLine: null });
  }

  return lines;
}

function turnChangeFileKey(file: TurnFileChange) {
  return `${file.kind}:${file.oldPath ?? ""}:${file.newPath ?? ""}`;
}

function turnChangeFileRequestPath(file: TurnFileChange) {
  return file.newPath ?? file.oldPath ?? null;
}

function turnFilePreviewError(error: unknown) {
  const message = error instanceof Error ? error.message : String(error);
  try {
    const parsed = JSON.parse(message) as { error?: string };
    return parsed.error || message;
  } catch {
    return message || "代码差异加载失败，请重试。";
  }
}

function utf8ByteLength(value: string) {
  return new TextEncoder().encode(value).length;
}

function formatPreviewBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.ceil(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function turnChangeFilePath(file: TurnFileChange) {
  if (file.kind === "renamed" && file.oldPath && file.newPath) {
    return `${file.oldPath} → ${file.newPath}`;
  }
  return file.newPath ?? file.oldPath ?? "未知文件";
}

function turnChangeKind(kind: TurnFileChange["kind"]) {
  if (kind === "added") return { code: "A", label: "新增" };
  if (kind === "deleted") return { code: "D", label: "删除" };
  if (kind === "renamed") return { code: "R", label: "重命名" };
  return { code: "M", label: "修改" };
}

function ActivityEntryView({
  entry,
  isActive,
  now,
  formatError,
  onOpenMarkdownLink,
}: {
  entry: ActivityEntry;
  isActive: boolean;
  now: number;
  formatError(message: string): string;
  onOpenMarkdownLink?(href: string): void;
}) {
  if (entry.kind === "tool-group") {
    return (
      <ToolActivityGroup
        category={entry.category}
        executions={entry.executions}
        defaultExpanded={isActive}
        now={now}
      />
    );
  }
  if (entry.kind === "file-group") {
    return <FileActivityGroup files={entry.files} defaultExpanded={isActive} />;
  }
  if (entry.kind === "reasoning") {
    return (
      <NarrativeActivity
        kind="reasoning"
        onOpenMarkdownLink={onOpenMarkdownLink}
        streaming={isActive}
        text={entry.text}
      />
    );
  }
  if (entry.kind === "commentary") {
    return (
      <NarrativeActivity
        kind="commentary"
        onOpenMarkdownLink={onOpenMarkdownLink}
        streaming={isActive}
        text={entry.text}
      />
    );
  }
  if (entry.kind === "plan") {
    return (
      <PlanActivity
        plan={entry.plan}
        stepTimings={entry.stepTimings}
        startedAt={entry.startedAt}
        finishedAt={entry.finishedAt}
        defaultExpanded={isActive}
        isActive={isActive}
        now={now}
      />
    );
  }
  if (entry.kind === "subagent") {
    return <SubagentActivity run={entry.run} now={now} />;
  }
  if (entry.kind === "approval") {
    return (
      <ActivityNotice
        icon={<Clock3 size={13} />}
        tone="waiting"
        title="等待用户批准"
        detail={`${entry.reason}${entry.action ? `\n操作：${entry.action}` : ""}`}
      />
    );
  }
  if (entry.kind === "cancelled") {
    return (
      <ActivityNotice
        icon={<X size={13} />}
        tone="error"
        title="任务已取消"
        detail={entry.reason}
      />
    );
  }
  if (entry.kind === "suspended") {
    return (
      <ActivityNotice
        icon={<Clock3 size={13} />}
        tone="waiting"
        title="任务已暂停"
        detail={entry.reason}
      />
    );
  }
  return (
    <ActivityNotice
      icon={<AlertCircle size={13} />}
      tone="error"
      title="执行失败"
      detail={formatError(entry.message)}
    />
  );
}

function ToolActivityGroup({
  category,
  executions,
  defaultExpanded,
  now,
}: {
  category: ToolCategory;
  executions: ToolExecution[];
  defaultExpanded: boolean;
  now: number;
}) {
  const running = executions.some((execution) => !execution.result);
  const commandBatch = category === "command" && executions.length > 1;
  const runningCommand = [...executions]
    .reverse()
    .find((execution) => !execution.result);
  const failed = executions.some((execution) =>
    toolResultFailed(execution.result),
  );
  const timing = formatExecutionGroupTiming(executions, running, now);
  const [expanded, setExpanded] = useState(
    !commandBatch && (defaultExpanded || running),
  );
  const wasRunning = useRef(running);

  useEffect(() => {
    if (commandBatch) {
      if (!running && wasRunning.current) setExpanded(false);
      wasRunning.current = running;
      return;
    }
    if (running) setExpanded(true);
  }, [commandBatch, running]);

  if (executions.length === 1) {
    return <ToolExecutionItem execution={executions[0]} now={now} />;
  }

  if (commandBatch && running && runningCommand) {
    return (
      <ToolExecutionItem execution={runningCommand} now={now} currentCommand />
    );
  }

  return (
    <div
      className="activity-group"
      data-state={failed ? "error" : running ? "running" : "complete"}
    >
      <button
        className="activity-group-header"
        type="button"
        aria-expanded={expanded}
        onClick={() => setExpanded((current) => !current)}
      >
        <span className="activity-group-icon" aria-hidden="true">
          {toolCategoryIcon(category)}
        </span>
        <span>
          {toolGroupTitle(category, executions.length)}
          {timing ? ` · ${timing}` : ""}
        </span>
        <ActivityResultIcon running={running} failed={failed} />
        {expanded ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
      </button>
      {expanded && (
        <div className="activity-group-content">
          {executions.map((execution) => (
            <ToolExecutionItem
              key={execution.call.id}
              execution={execution}
              now={now}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function ToolExecutionItem({
  execution,
  now,
  currentCommand = false,
}: {
  execution: ToolExecution;
  now: number;
  currentCommand?: boolean;
}) {
  const running = !execution.result;
  const failed = toolResultFailed(execution.result);
  const [expanded, setExpanded] = useState(false);
  const timing = formatActivityTiming(
    execution.startedAt,
    execution.finishedAt,
    running,
    now,
  );
  const fileChanges = toolFileChangeSummaries(execution);
  const primaryFileChange =
    fileChanges.length === 1 ? fileChanges[0] : undefined;
  const input = formatToolInput(execution.call, fileChanges.length > 0);
  const output = formatToolOutput(execution.result);
  const sandbox = formatToolSandbox(execution.result);
  const title = primaryFileChange
    ? primaryFileChange.path
    : fileChanges.length > 1
      ? `修改了 ${fileChanges.length} 个文件`
      : toolExecutionTitle(execution.call);

  useEffect(() => {
    if (currentCommand) setExpanded(false);
  }, [currentCommand, execution.call.id]);

  return (
    <div
      className="tool-execution"
      data-state={failed ? "error" : running ? "running" : "complete"}
      data-current-command={currentCommand || undefined}
    >
      <button
        className="tool-execution-header"
        type="button"
        aria-expanded={expanded}
        onClick={() => setExpanded((current) => !current)}
      >
        <span className="tool-execution-state" aria-hidden="true">
          {currentCommand ? (
            <TerminalSquare size={12} />
          ) : (
            <ActivityResultIcon running={running} failed={failed} />
          )}
        </span>
        <span className="tool-execution-title">
          <strong
            className={currentCommand ? "tool-execution-title-flow" : undefined}
            title={primaryFileChange?.path}
          >
            {title}
          </strong>
          <small className="tool-execution-meta">
            <span>
              {primaryFileChange?.operation ||
                toolDisplayName(execution.call.name)}
            </span>
            {primaryFileChange && (
              <FileChangeStatsView change={primaryFileChange} />
            )}
            {timing && <span>· {timing}</span>}
            {sandbox && (
              <span
                className="tool-sandbox-state"
                data-tone={sandbox.unsafe ? "warning" : "neutral"}
                title={sandbox.detail}
              >
                · {sandbox.label}
              </span>
            )}
          </small>
        </span>
        {expanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
      </button>
      {fileChanges.length > 1 && (
        <FileChangeSummaryList changes={fileChanges} />
      )}
      {expanded && (
        <div className="tool-execution-details">
          <div>
            <span>{execution.call.name === "shell" ? "命令" : "参数"}</span>
            <pre>{input}</pre>
          </div>
          <div>
            <span>结果</span>
            <pre>{output}</pre>
          </div>
          {execution.call.name === "shell" && !running && (
            <footer
              className="tool-execution-shell-status"
              data-state={failed ? "error" : "success"}
            >
              {failed ? (
                <X size={12} aria-hidden="true" />
              ) : (
                <CheckCircle2 size={12} aria-hidden="true" />
              )}
              <span>{failed ? "失败" : "成功"}</span>
            </footer>
          )}
        </div>
      )}
    </div>
  );
}

function FileActivityGroup({
  files,
  defaultExpanded,
}: {
  files: ActivityFile[];
  defaultExpanded: boolean;
}) {
  const [expanded, setExpanded] = useState(defaultExpanded);
  const timing = formatFileGroupTiming(files);
  return (
    <div className="activity-group" data-state="complete">
      <button
        className="activity-group-header"
        type="button"
        aria-expanded={expanded}
        onClick={() => setExpanded((current) => !current)}
      >
        <span className="activity-group-icon" aria-hidden="true">
          <FileText size={13} />
        </span>
        <span>
          修改了 {files.length} 个文件{timing ? ` · ${timing}` : ""}
        </span>
        <ActivityResultIcon running={false} failed={false} />
        {expanded ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
      </button>
      {expanded && (
        <div className="activity-file-list">
          {files.map((file, index) => (
            <FileActivityItem key={`${file.path}-${index}`} file={file} />
          ))}
        </div>
      )}
    </div>
  );
}

function FileActivityItem({ file }: { file: ActivityFile }) {
  const [expanded, setExpanded] = useState(false);
  const detail = file.summary.trim();
  const change = fileChangedEventSummary(file);

  return (
    <div className="activity-file-item">
      <button
        type="button"
        className="activity-file-row"
        aria-expanded={detail ? expanded : undefined}
        onClick={() => detail && setExpanded((current) => !current)}
      >
        <FileText size={12} aria-hidden="true" />
        <span title={file.path}>{file.path}</span>
        <span className="activity-file-meta">
          <span>{change.operation}</span>
          <FileChangeStatsView change={change} />
        </span>
        {detail ? (
          expanded ? (
            <ChevronDown size={12} aria-hidden="true" />
          ) : (
            <ChevronRight size={12} aria-hidden="true" />
          )
        ) : (
          <span />
        )}
      </button>
      {expanded && detail && <p className="activity-file-detail">{detail}</p>}
    </div>
  );
}

function FileChangeSummaryList({ changes }: { changes: FileChangeSummary[] }) {
  return (
    <div className="file-change-summary-list" role="list" aria-label="文件变更">
      {changes.map((change, index) => (
        <div key={`${change.path}-${index}`} role="listitem">
          <FileText size={12} aria-hidden="true" />
          <span title={change.path}>{change.path}</span>
          <span className="activity-file-meta">
            <span>{change.operation}</span>
            <FileChangeStatsView change={change} />
          </span>
        </div>
      ))}
    </div>
  );
}

function FileChangeStatsView({ change }: { change: FileChangeSummary }) {
  if (change.additions === undefined && change.deletions === undefined) {
    return null;
  }
  return (
    <span
      className="file-change-stats"
      aria-label={fileChangeStatsLabel(change)}
    >
      {change.additions !== undefined && (
        <span className="file-change-additions">+{change.additions}</span>
      )}
      {change.deletions !== undefined && (
        <span className="file-change-deletions">-{change.deletions}</span>
      )}
    </span>
  );
}

function NarrativeActivity({
  kind,
  onOpenMarkdownLink,
  streaming,
  text,
}: {
  kind: "reasoning" | "commentary";
  onOpenMarkdownLink?(href: string): void;
  streaming: boolean;
  text: string;
}) {
  return (
    <div
      className="activity-narrative"
      data-kind={kind}
      aria-label={kind === "reasoning" ? "思考过程" : "处理说明"}
    >
      <MarkdownContent
        className="activity-narrative-markdown"
        onOpenLink={onOpenMarkdownLink}
        streaming={streaming}
        text={redactText(text)}
      />
    </div>
  );
}

function PlanActivity({
  plan,
  stepTimings,
  startedAt,
  finishedAt,
  defaultExpanded,
  isActive,
  now,
}: {
  plan: TaskPlan;
  stepTimings: PlanStepTiming[];
  startedAt: string;
  finishedAt?: string;
  defaultExpanded: boolean;
  isActive: boolean;
  now: number;
}) {
  const resolved = plan.steps.filter((step) =>
    ["completed", "deferred", "blocked", "cancelled"].includes(step.status),
  ).length;
  const running = plan.steps.some((step) => step.status === "in_progress");
  const actionable = plan.steps.some(
    (step) => step.status === "pending" || step.status === "in_progress",
  );
  const completedIds = new Set(
    plan.steps
      .filter((step) => step.status === "completed")
      .map((step) => step.id),
  );
  let currentStepIndex = plan.steps.findIndex(
    (step) => step.status === "in_progress",
  );
  if (currentStepIndex < 0) {
    currentStepIndex = plan.steps.findIndex(
      (step) =>
        step.status === "pending" &&
        step.dependencies.every((dependency) => completedIds.has(dependency)),
    );
  }
  const timing = formatActivityTiming(
    startedAt,
    finishedAt,
    running && isActive,
    now,
  );
  const [expanded, setExpanded] = useState(defaultExpanded || actionable);
  return (
    <div
      className="activity-group"
      data-state={actionable ? "running" : "complete"}
    >
      <button
        className="activity-group-header"
        type="button"
        aria-expanded={expanded}
        onClick={() => setExpanded((current) => !current)}
      >
        <span className="activity-group-icon" aria-hidden="true">
          <Activity size={13} />
        </span>
        <span>执行计划</span>
        <small className="activity-group-count">
          {currentStepIndex >= 0
            ? `第 ${currentStepIndex + 1}/${plan.steps.length} 步`
            : `${resolved}/${plan.steps.length} 已处理`}
          {timing ? ` · ${timing}` : ""}
        </small>
        {expanded ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
      </button>
      {expanded && (
        <div className="activity-plan">
          {(plan.changeReason || plan.explanation) && (
            <p>{plan.changeReason || plan.explanation}</p>
          )}
          <ol>
            {plan.steps.map((step, index) => {
              const title = step.title || step.step || step.id;
              const stepTiming = formatPlanStepTiming(
                stepTimings[index],
                step.status,
                isActive,
                now,
              );
              return (
                <li
                  key={step.id || `${index}-${title}`}
                  data-status={step.status}
                >
                  <span aria-hidden="true">
                    {step.status === "completed" ? (
                      <span className="activity-plan-dot is-complete" />
                    ) : step.status === "in_progress" ? (
                      <ActivityFlow compact />
                    ) : step.status === "blocked" ? (
                      <AlertCircle size={12} />
                    ) : step.status === "cancelled" ? (
                      <X size={12} />
                    ) : step.status === "deferred" ? (
                      <Clock3 size={12} />
                    ) : (
                      <span className="activity-plan-dot" />
                    )}
                  </span>
                  <span className="activity-plan-step-body">
                    <span>
                      {title}
                      {stepTiming ? ` · ${stepTiming}` : ""}
                    </span>
                    {step.statusReason && <small>{step.statusReason}</small>}
                  </span>
                </li>
              );
            })}
          </ol>
        </div>
      )}
    </div>
  );
}

function SubagentActivity({ run, now }: { run: SubagentRun; now: number }) {
  const [expanded, setExpanded] = useState(run.status === "running");
  const running = run.status === "running" || run.status === "queued";
  const failed = run.status === "failed" || run.status === "timed_out";
  const timing = formatActivityTiming(
    run.startedAt || run.createdAt,
    run.completedAt || undefined,
    running,
    now,
  );
  return (
    <div
      className="activity-group"
      data-state={failed ? "error" : running ? "running" : "complete"}
    >
      <button
        className="activity-group-header"
        type="button"
        aria-expanded={expanded}
        onClick={() => setExpanded((current) => !current)}
      >
        <span className="activity-group-icon" aria-hidden="true">
          <Bot size={13} />
        </span>
        <span>Agent：{run.agentPath || run.name}</span>
        <small className="activity-group-count">
          {subagentStatusLabel(run.status)}
          {timing ? ` · ${timing}` : ""}
        </small>
        {expanded ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
      </button>
      {expanded && (
        <div className="subagent-activity-details">
          <span>任务</span>
          <p>{redactText(run.input)}</p>
          {(run.result || run.error) && (
            <span>{run.error ? "错误" : "结果"}</span>
          )}
          {run.result && (
            <pre>{truncateText(redactText(run.result), 12_000)}</pre>
          )}
          {run.error && (
            <pre>{truncateText(redactText(run.error), 12_000)}</pre>
          )}
        </div>
      )}
    </div>
  );
}

function ActivityNotice({
  icon,
  title,
  detail,
  tone = "neutral",
}: {
  icon: ReactNode;
  title: string;
  detail: string;
  tone?: "neutral" | "waiting" | "error";
}) {
  return (
    <div className="activity-notice" data-tone={tone}>
      <span aria-hidden="true">{icon}</span>
      <div>
        <strong>{title}</strong>
        <p>{detail}</p>
      </div>
    </div>
  );
}

function ActivityResultIcon({
  running,
  failed,
}: {
  running: boolean;
  failed: boolean;
}) {
  if (running) return <ActivityFlow compact />;
  if (failed) return <X size={12} className="activity-result-icon" />;
  return <span className="activity-result-spacer" aria-hidden="true" />;
}

function ActivityFlow({ compact = false }: { compact?: boolean }) {
  return (
    <span
      className={`activity-flow${compact ? " is-compact" : ""}`}
      aria-hidden="true"
    />
  );
}

function activityStateLabel(state: ActivityState, isActive: boolean) {
  if (isActive) return "处理中";
  if (state === "error") return "执行失败";
  if (state === "cancelled") return "已取消";
  if (state === "waiting") return "等待继续";
  return "已处理";
}

function buildActivityEntries(events: AgentEvent[]): ActivityEntry[] {
  const sorted = [...events].sort((left, right) => left.seq - right.seq);
  const finalResponseDeltaSeqs = findFinalResponseDeltaSeqs(sorted);
  const resultEvents = new Map<
    string,
    { result: ToolResult; createdAt: string; seq: number }
  >();
  const startedCallIds = new Set<string>();
  const subagents = new Map<
    string,
    Extract<PrimitiveActivity, { kind: "subagent" }>
  >();
  const primitives: PrimitiveActivity[] = [];
  const planEvents = sorted.filter(
    (event) => event.payload.type === "plan_updated",
  );
  const firstPlanEvent = planEvents[0];
  const latestPlanEvent = planEvents[planEvents.length - 1];
  const latestPlan =
    latestPlanEvent?.payload.type === "plan_updated"
      ? latestPlanEvent.payload.plan
      : undefined;
  const planStepTimings = latestPlan
    ? buildPlanStepTimings(planEvents, latestPlan)
    : [];

  for (const event of sorted) {
    if (event.payload.type === "tool_call_finished") {
      resultEvents.set(event.payload.result.callId, {
        result: event.payload.result,
        createdAt: event.createdAt,
        seq: event.seq,
      });
    }
  }

  for (const event of sorted) {
    const payload = event.payload;
    const reasoning = readReasoningPayload(payload);
    if (reasoning) {
      primitives.push({
        kind: "reasoning",
        seq: event.seq,
        text: reasoning.text,
        isDelta: reasoning.isDelta,
        createdAt: event.createdAt,
      });
    } else if (
      payload.type === "model_delta" &&
      !finalResponseDeltaSeqs.has(event.seq) &&
      payload.text.length > 0
    ) {
      primitives.push({
        kind: "commentary",
        seq: event.seq,
        text: payload.text,
        createdAt: event.createdAt,
      });
    } else if (payload.type === "tool_call_started") {
      startedCallIds.add(payload.call.id);
      const finished = resultEvents.get(payload.call.id);
      primitives.push({
        kind: "tool",
        seq: event.seq,
        execution: {
          call: payload.call,
          startedAt: event.createdAt,
          result: finished?.result,
          finishedAt: finished?.createdAt,
        },
      });
    } else if (
      payload.type === "plan_updated" &&
      event.id === firstPlanEvent?.id &&
      latestPlanEvent?.payload.type === "plan_updated"
    ) {
      primitives.push({
        kind: "plan",
        seq: event.seq,
        plan: latestPlanEvent.payload.plan,
        startedAt: event.createdAt,
        finishedAt: latestPlanEvent.payload.plan.steps.every(
          (step) => step.status === "completed",
        )
          ? latestPlanEvent.createdAt
          : undefined,
        stepTimings: planStepTimings,
      });
    } else if (payload.type === "file_changed") {
      primitives.push({
        kind: "file",
        seq: event.seq,
        path: payload.path,
        summary: payload.summary,
        createdAt: event.createdAt,
      });
    } else if (payload.type === "subagent_updated") {
      const current = subagents.get(payload.run.id);
      if (current) {
        current.run = payload.run;
      } else {
        const entry: Extract<PrimitiveActivity, { kind: "subagent" }> = {
          kind: "subagent",
          seq: event.seq,
          run: payload.run,
          createdAt: event.createdAt,
        };
        subagents.set(payload.run.id, entry);
        primitives.push(entry);
      }
    } else if (payload.type === "approval_requested") {
      primitives.push({
        kind: "approval",
        seq: event.seq,
        reason: payload.reason,
        action: payload.action,
        createdAt: event.createdAt,
      });
    } else if (payload.type === "error") {
      primitives.push({
        kind: "error",
        seq: event.seq,
        message: payload.message,
        createdAt: event.createdAt,
      });
    } else if (payload.type === "turn_cancelled") {
      primitives.push({
        kind: "cancelled",
        seq: event.seq,
        reason: payload.reason,
        createdAt: event.createdAt,
      });
    } else if (payload.type === "turn_suspended") {
      primitives.push({
        kind: "suspended",
        seq: event.seq,
        reason: payload.reason,
        createdAt: event.createdAt,
      });
    }
  }

  for (const [callId, finished] of resultEvents) {
    if (startedCallIds.has(callId)) continue;
    const metadata = asRecord(finished.result.metadata);
    primitives.push({
      kind: "tool",
      seq: finished.seq,
      execution: {
        call: {
          id: callId,
          name:
            typeof metadata?.toolName === "string" ? metadata.toolName : "tool",
          input: {},
        },
        startedAt: finished.createdAt,
        result: finished.result,
        finishedAt: finished.createdAt,
      },
    });
  }

  primitives.sort((left, right) => left.seq - right.seq);
  const entries: ActivityEntry[] = [];
  for (const primitive of primitives) {
    if (primitive.kind === "tool") {
      const category = toolCategory(primitive.execution.call.name);
      const previous = entries[entries.length - 1];
      if (previous?.kind === "tool-group" && previous.category === category) {
        previous.executions.push(primitive.execution);
      } else {
        entries.push({
          kind: "tool-group",
          id: `tool-${primitive.seq}`,
          category,
          executions: [primitive.execution],
        });
      }
    } else if (primitive.kind === "file") {
      const previous = entries[entries.length - 1];
      if (previous?.kind === "file-group") {
        previous.files.push({
          path: primitive.path,
          summary: primitive.summary,
          createdAt: primitive.createdAt,
        });
      } else {
        entries.push({
          kind: "file-group",
          id: `file-${primitive.seq}`,
          files: [
            {
              path: primitive.path,
              summary: primitive.summary,
              createdAt: primitive.createdAt,
            },
          ],
        });
      }
    } else if (primitive.kind === "reasoning") {
      const previous = entries[entries.length - 1];
      if (previous?.kind === "reasoning") {
        previous.text = appendReasoningText(
          previous.text,
          primitive.text,
          primitive.isDelta,
        );
      } else {
        entries.push(primitive);
      }
    } else if (primitive.kind === "commentary") {
      const previous = entries[entries.length - 1];
      if (previous?.kind === "commentary") {
        previous.text = appendReasoningText(
          previous.text,
          primitive.text,
          true,
        );
      } else {
        entries.push(primitive);
      }
    } else {
      entries.push(primitive);
    }
  }
  return entries;
}

function findFinalResponseDeltaSeqs(events: AgentEvent[]): Set<number> {
  let assistantIndex = -1;
  for (let index = events.length - 1; index >= 0; index -= 1) {
    if (events[index].payload.type === "assistant_message") {
      assistantIndex = index;
      break;
    }
  }
  if (assistantIndex < 0) return new Set();

  const assistantEvent = events[assistantIndex];
  if (assistantEvent.payload.type !== "assistant_message") return new Set();
  const finalText = assistantEvent.payload.message.parts
    .filter((part) => part.type === "text")
    .map((part) => (part.type === "text" ? part.text : ""))
    .join("");
  if (!finalText) return new Set();

  let requestIndex = -1;
  for (let index = assistantIndex - 1; index >= 0; index -= 1) {
    if (events[index].payload.type === "model_request") {
      requestIndex = index;
      break;
    }
  }
  if (requestIndex < 0) return new Set();

  let remaining = finalText;
  const matched = new Set<number>();
  for (const event of events.slice(requestIndex + 1, assistantIndex)) {
    if (event.payload.type !== "model_delta") continue;
    if (!remaining.startsWith(event.payload.text)) return new Set();
    remaining = remaining.slice(event.payload.text.length);
    matched.add(event.seq);
    if (!remaining) return matched;
  }
  return new Set();
}

function buildPlanStepTimings(
  planEvents: AgentEvent[],
  latestPlan: TaskPlan,
): PlanStepTiming[] {
  return latestPlan.steps.map((latestStep, stepIndex) => {
    let firstSeenAt: string | undefined;
    let startedAt: string | undefined;
    let finishedAt: string | undefined;

    for (const event of planEvents) {
      if (event.payload.type !== "plan_updated") continue;
      const snapshotStep =
        event.payload.plan.steps.find(
          (step) =>
            (step.id && step.id === latestStep.id) ||
            (step.title || step.step) === (latestStep.title || latestStep.step),
        ) ?? event.payload.plan.steps[stepIndex];
      if (!snapshotStep) continue;

      firstSeenAt ??= event.createdAt;
      if (snapshotStep.status === "in_progress") {
        startedAt ??= event.createdAt;
      } else if (snapshotStep.status === "completed") {
        startedAt ??= firstSeenAt;
        finishedAt ??= event.createdAt;
      }
    }

    if (latestStep.status === "in_progress") {
      startedAt ??= firstSeenAt;
    }
    return { startedAt, finishedAt };
  });
}

function useTimelineClock(shouldTick: boolean) {
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    setNow(Date.now());
    if (!shouldTick) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1_000);
    return () => window.clearInterval(timer);
  }, [shouldTick]);

  return now;
}

function activityEntryIsRunning(entry: ActivityEntry) {
  if (entry.kind === "tool-group") {
    return entry.executions.some((execution) => !execution.result);
  }
  if (entry.kind === "plan") {
    return entry.plan.steps.some((step) => step.status === "in_progress");
  }
  if (entry.kind === "subagent") {
    return entry.run.status === "queued" || entry.run.status === "running";
  }
  return false;
}

function activityState(events: AgentEvent[], isActive: boolean): ActivityState {
  if (isActive) return "running";
  const terminal = [...events]
    .sort((left, right) => right.seq - left.seq)
    .find((event) =>
      ["turn_finished", "turn_cancelled", "turn_suspended", "error"].includes(
        event.payload.type,
      ),
    )?.payload;
  if (terminal?.type === "error") return "error";
  if (terminal?.type === "turn_cancelled") return "cancelled";
  if (terminal?.type === "turn_suspended") return "waiting";
  return "complete";
}

function activityEntryKey(entry: ActivityEntry) {
  if (entry.kind === "tool-group" || entry.kind === "file-group")
    return entry.id;
  if (entry.kind === "subagent") return `subagent-${entry.run.id}`;
  return `${entry.kind}-${entry.seq}`;
}

function toolCategory(name: string): ToolCategory {
  if (name === "shell") return "command";
  if (
    [
      "list_files",
      "read_file",
      "write_file",
      "search",
      "git_diff",
      "apply_patch",
    ].includes(name)
  ) {
    return "file";
  }
  if (name === "browser") return "browser";
  if (name === "spreadsheet") return "spreadsheet";
  if (
    [
      "spawn_agent",
      "send_input",
      "cancel_agent",
      "wait_agent",
      "wait_agents",
    ].includes(name)
  ) {
    return "agent";
  }
  return "tool";
}

function toolCategoryIcon(category: ToolCategory) {
  if (category === "command") return <TerminalSquare size={13} />;
  if (category === "file") return <FileText size={13} />;
  if (category === "browser") return <Globe2 size={13} />;
  if (category === "spreadsheet") return <Table2 size={13} />;
  if (category === "agent") return <Bot size={13} />;
  return <Wrench size={13} />;
}

function toolGroupTitle(category: ToolCategory, count: number) {
  if (category === "command") return "运行了多个命令";
  if (category === "file") return `进行了 ${count} 个文件操作`;
  if (category === "browser") return `进行了 ${count} 个浏览器操作`;
  if (category === "spreadsheet") return `进行了 ${count} 个表格操作`;
  if (category === "agent") return `进行了 ${count} 个子智能体操作`;
  return `调用了 ${count} 个工具`;
}

function toolExecutionTitle(call: ToolCall) {
  const input = asRecord(call.input);
  if (call.name === "shell") {
    return truncateLine(stringField(input, "command") || "运行命令", 140);
  }
  if (call.name === "list_files")
    return `查看文件 ${stringField(input, "path") || "."}`;
  if (call.name === "read_file")
    return `读取 ${stringField(input, "path") || "文件"}`;
  if (call.name === "write_file")
    return `写入 ${stringField(input, "path") || "文件"}`;
  if (call.name === "search") {
    return `搜索 ${stringField(input, "query") || stringField(input, "pattern") || "内容"}`;
  }
  if (call.name === "git_diff") return "检查 Git 变更";
  if (call.name === "apply_patch")
    return `修改文件${patchTarget(input) ? ` ${patchTarget(input)}` : ""}`;
  if (call.name === "browser") {
    const action = stringField(input, "action") || "操作";
    const target = stringField(input, "url") || stringField(input, "selector");
    return `浏览器 ${action}${target ? ` · ${truncateLine(target, 90)}` : ""}`;
  }
  if (call.name === "spreadsheet") {
    const action = stringField(input, "action") || "操作";
    const target =
      stringField(input, "path") || stringField(input, "outputPath");
    return `表格 ${action}${target ? ` · ${target}` : ""}`;
  }
  if (call.name === "spawn_agent")
    return `创建子智能体 ${stringField(input, "name") || ""}`.trim();
  if (call.name === "send_input") return "向子智能体发送消息";
  if (call.name === "wait_agent" || call.name === "wait_agents")
    return "等待子智能体完成";
  if (call.name === "cancel_agent") return "取消子智能体";
  if (call.name === "update_plan") return "更新执行计划";
  if (call.name === "complete_task") return "完成任务";
  if (call.name === "list_skills") return "查看可用 Skill";
  if (call.name === "read_skill")
    return `读取 Skill ${stringField(input, "name") || ""}`.trim();
  const mcpTool = mcpToolNameParts(call.name);
  if (mcpTool) return `MCP · ${mcpTool.server} / ${mcpTool.tool}`;
  return call.name;
}

function mcpToolNameParts(
  name: string,
): { server: string; tool: string } | null {
  const separator = name.indexOf("__");
  if (separator <= 0 || separator >= name.length - 2) return null;
  return {
    server: name.slice(0, separator),
    tool: name.slice(separator + 2),
  };
}

function toolDisplayName(name: string) {
  const names: Record<string, string> = {
    shell: "Shell",
    list_files: "文件列表",
    read_file: "读取文件",
    write_file: "写入文件",
    search: "代码搜索",
    git_diff: "Git Diff",
    apply_patch: "Apply Patch",
    browser: "浏览器",
    spreadsheet: "Excel",
    spawn_agent: "子智能体",
    send_input: "子智能体",
    wait_agent: "子智能体",
    wait_agents: "子智能体",
    cancel_agent: "子智能体",
    update_plan: "任务计划",
    complete_task: "任务闭环",
  };
  return names[name] || (mcpToolNameParts(name) ? "MCP" : name);
}

function toolFileChangeSummaries(
  execution: ToolExecution,
): FileChangeSummary[] {
  const input = asRecord(execution.call.input);
  const metadataChanges = fileChangesFromMetadata(execution.result?.metadata);

  if (execution.call.name === "write_file") {
    const inputPath = stringField(input, "path");
    const metadataChange = findMatchingFileChange(metadataChanges, inputPath);
    const path = inputPath || metadataChange?.path;
    if (!path) return metadataChanges;
    return [
      {
        path,
        operation: metadataChange?.operation || "写入",
        additions: metadataChange?.additions,
        deletions: metadataChange?.deletions,
      },
    ];
  }

  const diffText =
    execution.call.name === "apply_patch"
      ? stringField(input, "patch")
      : execution.call.name === "git_diff"
        ? execution.result?.output || ""
        : "";
  const diffChanges = parseUnifiedDiffChanges(diffText);
  if (diffChanges.length > 0) {
    return diffChanges.map((change) => {
      const metadataChange = findMatchingFileChange(
        metadataChanges,
        change.path,
      );
      return metadataChange
        ? mergeFileChange(change, metadataChange, change.path)
        : change;
    });
  }
  if (metadataChanges.length > 0) return metadataChanges;

  if (execution.call.name === "apply_patch") {
    const path = patchTarget(input);
    return path ? [{ path, operation: "修改" }] : [];
  }
  return [];
}

function fileChangedEventSummary(file: ActivityFile): FileChangeSummary {
  const stats = parseSummaryLineStats(file.summary);
  return {
    path: file.path,
    operation: fileOperationLabel(file.summary, "修改"),
    ...stats,
    detail: file.summary.trim() || undefined,
  };
}

function fileChangesFromMetadata(value: unknown): FileChangeSummary[] {
  const metadata = asRecord(value);
  if (!metadata) return [];
  const candidates: Record<string, unknown>[] = [];
  for (const key of ["fileChanges", "changes", "files"]) {
    const list = metadata[key];
    if (!Array.isArray(list)) continue;
    for (const item of list) {
      const record = asRecord(item);
      if (record) candidates.push(record);
    }
  }
  if (
    ["changedPath", "path", "filePath"].some(
      (key) => typeof metadata[key] === "string",
    )
  ) {
    candidates.push(metadata);
  }

  return candidates.flatMap((record) => {
    const path = firstStringField(record, [
      "changedPath",
      "path",
      "filePath",
      "file",
    ]);
    if (!path) return [];
    const additions = firstLineCount(record, [
      "additions",
      "addedLines",
      "linesAdded",
      "insertions",
    ]);
    const deletions = firstLineCount(record, [
      "deletions",
      "deletedLines",
      "removedLines",
      "linesDeleted",
    ]);
    const operation = fileOperationLabel(
      firstStringField(record, ["operation", "action", "status"]),
      "修改",
    );
    return [{ path, operation, additions, deletions }];
  });
}

function parseUnifiedDiffChanges(value: string): FileChangeSummary[] {
  if (!value.trim()) return [];
  type MutableChange = FileChangeSummary & { statsKnown: boolean };
  const changes: MutableChange[] = [];
  let current: MutableChange | undefined;
  let oldPath = "";
  let inHunk = false;

  const selectChange = (
    path: string,
    operation = "修改",
    statsKnown = false,
  ) => {
    const normalizedPath = cleanDiffPath(path);
    if (!normalizedPath || normalizedPath === "/dev/null") return undefined;
    const key = normalizeFileChangeKey(normalizedPath);
    current = changes.find(
      (change) => normalizeFileChangeKey(change.path) === key,
    );
    if (!current) {
      current = {
        path: normalizedPath,
        operation,
        additions: 0,
        deletions: 0,
        statsKnown,
      };
      changes.push(current);
    } else {
      current.operation = operation || current.operation;
      current.statsKnown ||= statsKnown;
    }
    return current;
  };

  for (const line of value.split(/\r?\n/)) {
    const customHeader = line.match(
      /^\*\*\* (Add|Update|Delete) File:\s*(.+)$/,
    );
    if (customHeader) {
      const operation =
        customHeader[1] === "Add"
          ? "新建"
          : customHeader[1] === "Delete"
            ? "删除"
            : "修改";
      selectChange(customHeader[2], operation, true);
      inHunk = true;
      continue;
    }

    const diffHeader = line.match(/^diff --git\s+\S+\s+(\S+)$/);
    if (diffHeader) {
      selectChange(diffHeader[1]);
      oldPath = "";
      inHunk = false;
      continue;
    }
    if (line.startsWith("--- ")) {
      oldPath = cleanDiffPath(line.slice(4));
      inHunk = false;
      continue;
    }
    if (line.startsWith("+++ ")) {
      const newPath = cleanDiffPath(line.slice(4));
      const deleting = newPath === "/dev/null";
      selectChange(
        deleting ? oldPath : newPath,
        deleting ? "删除" : oldPath === "/dev/null" ? "新建" : "修改",
      );
      continue;
    }
    if (line.startsWith("@@")) {
      if (current) current.statsKnown = true;
      inHunk = true;
      continue;
    }
    if (!current || !inHunk) continue;
    if (line.startsWith("+") && !line.startsWith("+++")) {
      current.additions = (current.additions || 0) + 1;
      current.statsKnown = true;
    } else if (line.startsWith("-") && !line.startsWith("---")) {
      current.deletions = (current.deletions || 0) + 1;
      current.statsKnown = true;
    }
  }

  return changes.map(({ statsKnown, ...change }) =>
    statsKnown ? change : { path: change.path, operation: change.operation },
  );
}

function parseSummaryLineStats(
  summary: string,
): Pick<FileChangeSummary, "additions" | "deletions"> {
  const compact = summary.replace(/,/g, "");
  const paired = compact.match(/(?:^|\s)\+(\d+)\s+(?:-|−)(\d+)(?:\s|$)/);
  if (paired) {
    return { additions: Number(paired[1]), deletions: Number(paired[2]) };
  }
  const additions = compact.match(/(\d+)\s+(?:insertions?|additions?)\b/i);
  const deletions = compact.match(/(\d+)\s+(?:deletions?|removals?)\b/i);
  return {
    additions: additions ? Number(additions[1]) : undefined,
    deletions: deletions ? Number(deletions[1]) : undefined,
  };
}

function findMatchingFileChange(changes: FileChangeSummary[], path: string) {
  if (!path) return changes[0];
  const key = normalizeFileChangeKey(path);
  return changes.find((change) => {
    const candidate = normalizeFileChangeKey(change.path);
    return (
      candidate === key ||
      candidate.endsWith(`/${key}`) ||
      key.endsWith(`/${candidate}`)
    );
  });
}

function mergeFileChange(
  base: FileChangeSummary,
  overlay: FileChangeSummary,
  path = base.path,
): FileChangeSummary {
  return {
    path,
    operation: overlay.operation || base.operation,
    additions: overlay.additions ?? base.additions,
    deletions: overlay.deletions ?? base.deletions,
    detail: overlay.detail || base.detail,
  };
}

function fileOperationLabel(value: string, fallback: string) {
  const normalized = value.toLowerCase();
  if (/\b(add|added|create|created|new)\b|新建|创建/.test(normalized))
    return "新建";
  if (/\b(delete|deleted|remove|removed)\b|删除/.test(normalized))
    return "删除";
  if (/\b(write|wrote|written)\b|写入/.test(normalized)) return "写入";
  if (/\b(revert|reverted|restore|restored)\b|回滚|恢复/.test(normalized))
    return "回滚";
  return fallback;
}

function cleanDiffPath(value: string) {
  const withoutTimestamp = value.trim().split("\t", 1)[0].replace(/^"|"$/g, "");
  if (withoutTimestamp === "/dev/null") return withoutTimestamp;
  return withoutTimestamp.replace(/^(?:a|b)[\\/]/, "").replace(/\\/g, "/");
}

function normalizeFileChangeKey(value: string) {
  return cleanDiffPath(value).replace(/^\.\//, "").toLowerCase();
}

function firstStringField(record: Record<string, unknown>, keys: string[]) {
  for (const key of keys) {
    if (typeof record[key] === "string" && record[key].trim()) {
      return record[key].trim();
    }
  }
  return "";
}

function firstLineCount(record: Record<string, unknown>, keys: string[]) {
  for (const key of keys) {
    const value = record[key];
    const count =
      typeof value === "number"
        ? value
        : typeof value === "string" && /^\d+$/.test(value)
          ? Number(value)
          : undefined;
    if (count !== undefined && Number.isFinite(count) && count >= 0) {
      return Math.floor(count);
    }
  }
  return undefined;
}

function fileChangeStatsLabel(change: FileChangeSummary) {
  const parts: string[] = [];
  if (change.additions !== undefined) parts.push(`新增 ${change.additions} 行`);
  if (change.deletions !== undefined) parts.push(`删除 ${change.deletions} 行`);
  return parts.join("，");
}

function readReasoningPayload(value: unknown) {
  const payload = asRecord(value);
  const type = stringField(payload, "type");
  if (
    ![
      "reasoning_delta",
      "reasoning_summary",
      "reasoning_summary_delta",
      "model_reasoning_delta",
      "thinking_delta",
    ].includes(type)
  ) {
    return null;
  }
  const nestedSummary = asRecord(payload?.summary);
  const rawText =
    stringField(payload, "text") ||
    (typeof payload?.summary === "string" ? payload.summary : "") ||
    stringField(nestedSummary, "text");
  if (!rawText.trim()) return null;
  const isDelta = type.endsWith("_delta");
  return { text: isDelta ? rawText : rawText.trim(), isDelta };
}

function appendReasoningText(previous: string, text: string, isDelta: boolean) {
  if (!previous) return text;
  if (previous === text || previous.endsWith(text)) return previous;
  return isDelta ? `${previous}${text}` : `${previous}\n${text}`;
}

function formatToolInput(call: ToolCall, hasFileSummary = false) {
  const input = asRecord(call.input);
  if (call.name === "shell") {
    return truncateText(
      redactText(stringField(input, "command") || "未提供命令"),
      20_000,
    );
  }
  let value: unknown = call.input;
  if (
    call.name === "write_file" &&
    input &&
    typeof input.content === "string"
  ) {
    value = {
      ...input,
      content: `[已隐藏文件正文，共 ${input.content.length} 个字符]`,
    };
  } else if (
    call.name === "apply_patch" &&
    input &&
    typeof input.patch === "string" &&
    !hasFileSummary
  ) {
    value = {
      ...input,
      patch: `[已隐藏补丁正文，共 ${input.patch.length} 个字符；未获得可靠的文件行数统计]`,
    };
  }
  return truncateText(
    JSON.stringify(sanitizeValue(value), null, 2) || "{}",
    20_000,
  );
}

function formatToolOutput(result?: ToolResult) {
  if (!result) return "等待工具返回...";
  const output = redactText(result.output || "").trim();
  return output
    ? truncateText(output, 20_000)
    : "工具执行完成，未返回文本输出。";
}

function formatToolSandbox(
  result?: ToolResult,
): { label: string; detail: string; unsafe: boolean } | null {
  if (!result) return null;
  const metadata = asRecord(result.metadata);
  const escalation = metadata?.sandboxEscalation;
  if (escalation === "scoped") {
    return {
      label: "scoped approval",
      detail: "This approval applied only to the replayed tool call.",
      unsafe: false,
    };
  }
  if (escalation === "denied") {
    return {
      label: "sandbox kept",
      detail: "The approval did not disable the configured OS sandbox.",
      unsafe: false,
    };
  }

  const sandbox = asRecord(metadata?.sandbox);
  if (!sandbox || typeof sandbox.status !== "string") return null;
  const profile =
    typeof sandbox.permissionProfile === "string"
      ? sandbox.permissionProfile
      : "unknown profile";
  if (sandbox.status === "wrapped") {
    const backend =
      typeof sandbox.backend === "string" ? sandbox.backend : "OS sandbox";
    return {
      label: `${backend} / ${profile}`,
      detail: `Wrapped by ${backend} with permission profile ${profile}.`,
      unsafe: false,
    };
  }
  if (sandbox.status === "best_effort_passthrough") {
    const reason =
      typeof sandbox.reason === "string"
        ? sandbox.reason
        : "The OS sandbox backend was unavailable.";
    return { label: "sandbox passthrough", detail: reason, unsafe: true };
  }
  if (sandbox.status === "unrestricted") {
    return {
      label: "unrestricted",
      detail: "This process ran with danger-full-access.",
      unsafe: true,
    };
  }
  return {
    label: "sandbox disabled",
    detail: `OS sandbox wrapping was disabled for profile ${profile}.`,
    unsafe: true,
  };
}

function toolResultFailed(result?: ToolResult) {
  if (!result) return false;
  const metadata = asRecord(result.metadata);
  return metadata?.success === false || metadata?.isError === true;
}

function formatTurnTiming(
  events: AgentEvent[],
  isActive: boolean,
  now: number,
  mountedAt: number,
) {
  const validEvents = [...events]
    .sort((left, right) => left.seq - right.seq)
    .map((event) => ({ event, time: parseTimestamp(event.createdAt) }))
    .filter(
      (item): item is { event: AgentEvent; time: number } => item.time !== null,
    );
  const turnStarted = validEvents.find(
    ({ event }) => event.payload.type === "turn_started",
  );
  const startedAt =
    turnStarted?.time ?? validEvents[0]?.time ?? (isActive ? mountedAt : null);
  if (startedAt === null || startedAt === undefined) return "";

  const terminal = [...validEvents]
    .reverse()
    .find(({ event }) =>
      ["turn_finished", "turn_cancelled", "turn_suspended", "error"].includes(
        event.payload.type,
      ),
    );
  const finishedAt = isActive
    ? now
    : (terminal?.time ?? validEvents[validEvents.length - 1]?.time ?? null);
  if (finishedAt === null || finishedAt < startedAt) return "";
  return formatTurnElapsed(finishedAt - startedAt);
}

function formatActivityTiming(
  startedAt?: string | null,
  finishedAt?: string | null,
  running = false,
  now = Date.now(),
) {
  const start = parseTimestamp(startedAt);
  if (start === null) return "";
  const finish = running ? now : parseTimestamp(finishedAt);
  if (finish === null || finish < start) return "";
  return `${running ? "已运行" : "耗时"} ${formatElapsed(finish - start)}`;
}

function formatExecutionGroupTiming(
  executions: ToolExecution[],
  running: boolean,
  now: number,
) {
  const starts = executions
    .map((execution) => parseTimestamp(execution.startedAt))
    .filter((value): value is number => value !== null);
  if (starts.length === 0) return "";
  const start = Math.min(...starts);
  const finishes = executions
    .map((execution) => parseTimestamp(execution.finishedAt))
    .filter((value): value is number => value !== null);
  const finish = running
    ? now
    : finishes.length > 0
      ? Math.max(...finishes)
      : null;
  return formatParsedTiming(start, finish, running);
}

function formatFileGroupTiming(files: Array<{ createdAt: string }>) {
  const times = files
    .map((file) => parseTimestamp(file.createdAt))
    .filter((value): value is number => value !== null);
  if (times.length === 0) return "";
  return formatParsedTiming(Math.min(...times), Math.max(...times), false);
}

function formatPlanStepTiming(
  timing: PlanStepTiming | undefined,
  status: TaskPlan["steps"][number]["status"],
  isActive: boolean,
  now: number,
) {
  if (status === "pending") return "尚未开始";
  const formatted = formatActivityTiming(
    timing?.startedAt,
    timing?.finishedAt,
    status === "in_progress" && isActive,
    now,
  );
  return formatted || "时间不可用";
}

function formatParsedTiming(
  startedAt: number,
  finishedAt: number | null,
  running: boolean,
) {
  if (finishedAt === null || finishedAt < startedAt) return "";
  return `${running ? "已运行" : "耗时"} ${formatElapsed(
    finishedAt - startedAt,
  )}`;
}

function parseTimestamp(value?: string | null) {
  if (!value) return null;
  const timestamp = Date.parse(value);
  return Number.isFinite(timestamp) ? timestamp : null;
}

function formatElapsed(duration: number) {
  const safeDuration = Math.max(0, Math.round(duration));
  if (safeDuration < 1_000) return `${safeDuration} ms`;
  if (safeDuration < 10_000) return `${(safeDuration / 1_000).toFixed(1)} 秒`;
  if (safeDuration < 60_000) return `${Math.round(safeDuration / 1_000)} 秒`;

  const totalSeconds = Math.round(safeDuration / 1_000);
  const seconds = totalSeconds % 60;
  const totalMinutes = Math.floor(totalSeconds / 60);
  const minutes = totalMinutes % 60;
  const hours = Math.floor(totalMinutes / 60);
  if (hours > 0) return `${hours} 小时 ${minutes} 分 ${seconds} 秒`;
  return `${minutes} 分 ${seconds} 秒`;
}

function formatTurnElapsed(duration: number) {
  const totalSeconds = Math.max(0, Math.round(duration / 1_000));
  const seconds = totalSeconds % 60;
  const totalMinutes = Math.floor(totalSeconds / 60);
  const minutes = totalMinutes % 60;
  const hours = Math.floor(totalMinutes / 60);
  if (hours > 0) return `${hours}h ${minutes}m ${seconds}s`;
  if (minutes > 0) return `${minutes}m ${seconds}s`;
  return `${seconds}s`;
}

function subagentStatusLabel(status: SubagentRun["status"]) {
  const labels: Record<SubagentRun["status"], string> = {
    queued: "排队中",
    running: "执行中",
    completed: "已完成",
    failed: "失败",
    cancelled: "已取消",
    timed_out: "超时",
  };
  return labels[status];
}

function patchTarget(input: Record<string, unknown> | null) {
  const patch = stringField(input, "patch");
  if (!patch) return "";
  return (
    patch.match(/^\*\*\* (?:Update|Add|Delete) File:\s*(.+)$/m)?.[1]?.trim() ||
    ""
  );
}

function stringField(record: Record<string, unknown> | null, key: string) {
  const value = record?.[key];
  return typeof value === "string" ? value : "";
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function sanitizeValue(value: unknown, key = "", depth = 0): unknown {
  if (/api[_-]?key|token|secret|password|authorization|credential/i.test(key)) {
    return "[已隐藏]";
  }
  if (depth > 8) return "[内容层级过深]";
  if (typeof value === "string") return redactText(value);
  if (Array.isArray(value)) {
    return value
      .slice(0, 100)
      .map((item) => sanitizeValue(item, key, depth + 1));
  }
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value as Record<string, unknown>).map(
        ([entryKey, item]) => [
          entryKey,
          sanitizeValue(item, entryKey, depth + 1),
        ],
      ),
    );
  }
  return value;
}

function redactText(value: string) {
  return value
    .replace(/(Bearer\s+)[^\s"'`]+/gi, "$1[已隐藏]")
    .replace(
      /((?:api[_-]?key|token|secret|password|authorization|credential)\s*[:=]\s*)("[^"]*"|'[^']*'|[^\s,;]+)/gi,
      "$1[已隐藏]",
    )
    .replace(/\bsk-[A-Za-z0-9_-]{8,}\b/g, "[已隐藏]");
}

function truncateLine(value: string, limit: number) {
  const line = value.replace(/\s+/g, " ").trim();
  return line.length <= limit ? line : `${line.slice(0, limit - 1)}…`;
}

function truncateText(value: string, limit: number) {
  return value.length <= limit
    ? value
    : `${value.slice(0, limit)}\n\n… 输出已截断，共 ${value.length} 个字符`;
}
