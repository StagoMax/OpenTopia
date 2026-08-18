import { useEffect, useState } from "react";
import {
  AlertCircle,
  ChevronDown,
  ChevronRight,
  ChevronUp,
  Code2,
  FileDiff,
  FileSearch,
  Loader2,
  RotateCcw,
} from "lucide-react";
import type {
  TurnChangeSet,
  TurnFileChange,
  TurnFileDiffPreview,
} from "../../types";
import {
  defaultVisibleDiffLines,
  formatPreviewBytes,
  parseTurnDiffLines,
  turnChangeFileKey,
  turnChangeFilePath,
  turnChangeFileRequestPath,
  turnChangeKind,
  turnFilePreviewError,
  utf8ByteLength,
  type TurnFilePreviewState,
} from "./turnChangePreview";

const defaultVisibleTurnFiles = 3;

export function TurnChangeCard({
  changeSet,
  isWorkspaceBusy = false,
  isUndoing = false,
  isReverted = false,
  onUndo,
  onReview,
  onLoadFilePreview,
  onOpenFileReview,
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
  // The caller decides whether a changed file belongs in the diff review or a
  // format-aware preview tab. Without this the row falls back to pinning.
  onOpenFileReview?(path: string, file: TurnFileChange): void;
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
            onSelect={
              onOpenFileReview
                ? () => {
                    const path = turnChangeFileRequestPath(file);
                    if (path) onOpenFileReview(path, file);
                  }
                : undefined
            }
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
  onSelect,
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
  onSelect?(): void;
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
        disabled={!canPreview && !onSelect}
        aria-expanded={expanded}
        aria-haspopup={onSelect ? undefined : "dialog"}
        aria-controls={canPreview ? previewId : undefined}
        aria-label={
          onSelect
            ? file.binary
              ? `在预览窗口中打开 ${path}`
              : `在审阅面板中打开 ${path}`
            : `${expanded ? "收起" : "预览"} ${path} 的代码差异`
        }
        onKeyDown={(event) => {
          if (event.key !== "Escape" || !expanded) return;
          event.preventDefault();
          setPinned(false);
          onClose();
        }}
        onClick={() => {
          // A caller-owned selection opens the appropriate destination while
          // the hover preview remains a separate affordance.
          if (onSelect) {
            setPinned(false);
            onClose();
            onSelect();
            return;
          }
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
