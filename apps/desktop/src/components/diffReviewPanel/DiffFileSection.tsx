import { useCallback, useMemo, useRef } from "react";
import {
  AlertCircle,
  ChevronDown,
  ChevronRight,
  ExternalLink,
  FileCode2,
  FileText,
  Loader2,
  RotateCcw,
  UnfoldVertical,
} from "lucide-react";
import {
  buildDiffBlocks,
  buildSplitRows,
  buildUnifiedRows,
  countDiffRows,
  diffFileDirectory,
  diffFileName,
  diffLanguageFromPath,
  fileAdditions,
  fileDeletions,
  type DiffBuildOptions,
  type DiffRowSide,
  type DiffSpan,
  type DiffSplitRow,
  type DiffUnifiedRow,
  type ParsedDiffFile,
} from "../../diffReview";
import type { DiffReviewPreferences } from "../../diffReviewPreferences";
import { MarkdownContent } from "../MarkdownContent";
import { Button, IconButton } from "../ui";
import {
  isRichPreviewPath,
  localGapIds,
  statusLabel,
  withLoadedContent,
  type ContentState,
  type DiffSplitPane,
} from "./model";

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

export function DeferredDiffFileSection({
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
  const rowsRef = useRef<HTMLDivElement>(null);
  const splitScrollContent = useMemo(
    () =>
      preferences.view === "split" && !preferences.wrapLines
        ? {
            left: splitScrollbarText(rows, "left"),
            right: splitScrollbarText(rows, "right"),
          }
        : null,
    [preferences.view, preferences.wrapLines, rows],
  );
  const setSplitScroll = useCallback(
    (pane: DiffSplitPane, scrollLeft: number) => {
      rowsRef.current?.style.setProperty(
        `--diff-review-${pane}-scroll`,
        `-${scrollLeft}px`,
      );
    },
    [],
  );

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
            ref={rowsRef}
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
                    <CodeCell side={row.left} pane="left" />
                    <LineNumber side={row.right} />
                    <CodeCell side={row.right} pane="right" />
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
          {splitScrollContent ? (
            <SplitScrollbars
              content={splitScrollContent}
              onScroll={setSplitScroll}
            />
          ) : null}
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

function CodeCell({
  side,
  pane,
}: {
  side: DiffRowSide | null;
  pane?: DiffSplitPane;
}) {
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
    <span
      className="diff-review__code"
      role="cell"
      data-kind={side.kind}
      data-pane={pane}
    >
      <span className="diff-review__code-content">
        <span className="diff-review__sign" aria-hidden="true">
          {side.kind === "added" ? "+" : side.kind === "removed" ? "-" : " "}
        </span>
        {side.spans.map((span, index) => (
          <span key={index} className={spanClassName(span)}>
            {span.text}
          </span>
        ))}
      </span>
    </span>
  );
}

function SplitScrollbars({
  content,
  onScroll,
}: {
  content: Record<DiffSplitPane, string>;
  onScroll(pane: DiffSplitPane, scrollLeft: number): void;
}) {
  return (
    <div
      className="diff-review__split-scrollbars"
      role="group"
      aria-label="分栏代码横向滚动条"
    >
      {(["left", "right"] as const).map((pane) => (
        <div
          className="diff-review__split-scrollbar"
          data-pane={pane}
          key={pane}
          tabIndex={0}
          title={pane === "left" ? "原始内容横向滚动" : "修改内容横向滚动"}
          aria-label={
            pane === "left" ? "原始内容横向滚动条" : "修改内容横向滚动条"
          }
          onScroll={(event) => onScroll(pane, event.currentTarget.scrollLeft)}
        >
          <span
            className="diff-review__split-scrollbar-track"
            aria-hidden="true"
          >
            {content[pane]}
          </span>
        </div>
      ))}
    </div>
  );
}

function splitScrollbarText(
  rows: Array<DiffSplitRow | DiffUnifiedRow>,
  pane: DiffSplitPane,
): string {
  return rows
    .map((row) => (row.type === "pair" ? (row[pane]?.text ?? "") : ""))
    .join("\n");
}

function spanClassName(span: DiffSpan): string {
  return [
    span.syntax ? `diff-review__token--${span.syntax}` : "",
    span.changed ? "is-changed" : "",
  ]
    .filter(Boolean)
    .join(" ");
}
