import {
  type MutableRefObject,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import DataEditor, {
  GridCellKind,
  getDefaultTheme,
  type Theme,
  type GridCell,
  type GridColumn,
  type Item,
  type Rectangle,
} from "@glideapps/glide-data-grid";
import "@glideapps/glide-data-grid/dist/index.css";
import { AlertCircle, FileQuestion, Loader2 } from "lucide-react";
import type { ApiClient } from "../api/client";
import type {
  PreviewDescriptor,
  SpreadsheetPreview,
  SpreadsheetPreviewCell,
  SpreadsheetPreviewRange,
} from "../types";
import {
  expandSpreadsheetWindow,
  spreadsheetWindowContains,
  type SpreadsheetWindow,
} from "../spreadsheetViewport";

const sheetRowHeight = 25;
const sheetColumnWidth = 120;
const sheetRowHeaderWidth = 48;
const sheetColumnHeaderHeight = 27;
const sheetRequestOverscan = { rows: 20, columns: 4 };
const sheetScrollSettleDelay = 120;
const maxCachedSheetRanges = 12;

type LoadState<T> =
  | { status: "loading" }
  | { status: "ready"; value: T }
  | { status: "error"; message: string };

type CachedSpreadsheetRange = {
  value: SpreadsheetPreviewRange;
  cells: Map<string, SpreadsheetPreviewCell>;
};

export function SpreadsheetGrid({
  client,
  descriptor,
}: {
  client: ApiClient;
  descriptor: PreviewDescriptor;
}) {
  const [book, setBook] = useState<LoadState<SpreadsheetPreview>>({
    status: "loading",
  });
  const [activeSheetId, setActiveSheetId] = useState<string | null>(null);
  const [cachedRanges, setCachedRanges] = useState<CachedSpreadsheetRange[]>(
    [],
  );
  const [requestedWindow, setRequestedWindow] =
    useState<SpreadsheetWindow | null>(null);
  const [rangeStatus, setRangeStatus] = useState<LoadState<null>>({
    status: "loading",
  });
  const [themeRevision, setThemeRevision] = useState(0);
  const visibleRegionRef = useRef<Rectangle | null>(null);
  const requestTimerRef = useRef<number | null>(null);
  const rangeRequestStartedRef = useRef(false);

  useEffect(() => {
    const observer = new MutationObserver(() =>
      setThemeRevision((current) => current + 1),
    );
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["data-theme"],
    });
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    let disposed = false;
    setBook({ status: "loading" });
    setActiveSheetId(null);
    setCachedRanges([]);
    setRequestedWindow(null);
    setRangeStatus({ status: "loading" });
    visibleRegionRef.current = null;
    rangeRequestStartedRef.current = false;
    void client
      .getSpreadsheetPreview(descriptor.threadId, descriptor.id)
      .then((value) => {
        if (disposed) return;
        setBook({ status: "ready", value });
        setActiveSheetId(value.sheets[0]?.id ?? null);
      })
      .catch((cause) => {
        if (!disposed)
          setBook({ status: "error", message: errorMessage(cause) });
      });
    return () => {
      disposed = true;
      clearRequestTimer(requestTimerRef);
    };
  }, [client, descriptor.id, descriptor.threadId]);

  const activeSheet =
    book.status === "ready"
      ? (book.value.sheets.find((sheet) => sheet.id === activeSheetId) ?? null)
      : null;

  const requestWindow =
    activeSheet && requestedWindow
      ? expandSpreadsheetWindow(
          requestedWindow,
          activeSheet,
          sheetRequestOverscan,
        )
      : null;
  const requestWindowCovered =
    activeSheetId !== null &&
    requestWindow !== null &&
    cachedRanges.some(
      ({ value }) =>
        value.sheetId === activeSheetId &&
        spreadsheetWindowContains(rangeWindow(value), requestWindow),
    );

  const cells = useMemo(() => {
    const next = new Map<string, SpreadsheetPreviewCell>();
    if (!activeSheetId) return next;
    for (const cached of cachedRanges) {
      if (cached.value.sheetId !== activeSheetId) continue;
      for (const [key, cell] of cached.cells) next.set(key, cell);
    }
    return next;
  }, [activeSheetId, cachedRanges]);

  const columns = useMemo<readonly GridColumn[]>(
    () =>
      activeSheet
        ? Array.from({ length: activeSheet.columnCount }, (_, index) => ({
            id: `column-${index}`,
            title: columnLabel(index),
            width: sheetColumnWidth,
          }))
        : [],
    [activeSheet],
  );

  const gridTheme = useMemo(() => buildGridTheme(), [themeRevision]);

  const getCellContent = useCallback(
    ([column, row]: Item): GridCell => {
      if (row < 0 || column < 0) {
        return { kind: GridCellKind.Loading, allowOverlay: false };
      }
      const cell = cells.get(`${row}:${column}`);
      if (!cell) {
        return {
          kind: GridCellKind.Loading,
          allowOverlay: false,
          skeletonWidth: 70,
        };
      }
      const text = formatCell(cell);
      return {
        kind: GridCellKind.Text,
        allowOverlay: true,
        data: text,
        displayData: text,
        readonly: true,
        copyData: text,
      };
    },
    [cells],
  );

  const onVisibleRegionChanged = useCallback(
    (region: Rectangle) => {
      if (!activeSheet || region.width <= 0 || region.height <= 0) return;
      visibleRegionRef.current = region;
      clearRequestTimer(requestTimerRef);
      const delay = rangeRequestStartedRef.current ? sheetScrollSettleDelay : 0;
      requestTimerRef.current = window.setTimeout(() => {
        requestTimerRef.current = null;
        const visible = visibleRegionRef.current;
        if (!visible) return;
        const next: SpreadsheetWindow = {
          rowStart: Math.max(0, Math.floor(visible.y)),
          rowCount: Math.max(1, Math.ceil(visible.height)),
          columnStart: Math.max(0, Math.floor(visible.x)),
          columnCount: Math.max(1, Math.ceil(visible.width)),
        };
        setRequestedWindow((current) =>
          current && sameSpreadsheetWindow(current, next) ? current : next,
        );
      }, delay);
    },
    [activeSheet],
  );

  useEffect(() => {
    if (!requestWindow || requestWindowCovered) {
      if (requestWindowCovered) {
        setRangeStatus((current) =>
          current.status === "ready"
            ? current
            : { status: "ready", value: null },
        );
      }
      return;
    }

    let disposed = false;
    const controller = new AbortController();
    setRangeStatus((current) =>
      current.status === "loading" ? current : { status: "loading" },
    );
    const timer = window.setTimeout(() => {
      rangeRequestStartedRef.current = true;
      void client
        .getSpreadsheetPreviewRange(
          descriptor.threadId,
          descriptor.id,
          activeSheetId!,
          {
            rowStart: requestWindow.rowStart,
            rowCount: requestWindow.rowCount,
            columnStart: requestWindow.columnStart,
            columnCount: requestWindow.columnCount,
          },
          controller.signal,
        )
        .then((value) => {
          if (disposed) return;
          const loadedCells = new Map<string, SpreadsheetPreviewCell>();
          for (const cell of value.cells)
            loadedCells.set(`${cell.row}:${cell.column}`, cell);
          setCachedRanges((current) => {
            const key = rangeKey(value);
            const next = [
              ...current.filter((cached) => rangeKey(cached.value) !== key),
              { value, cells: loadedCells },
            ];
            return next.slice(-maxCachedSheetRanges);
          });
          setRangeStatus({ status: "ready", value: null });
        })
        .catch((cause) => {
          if (disposed || isAbortError(cause)) return;
          setRangeStatus({ status: "error", message: errorMessage(cause) });
        });
    }, 0);

    return () => {
      disposed = true;
      window.clearTimeout(timer);
      controller.abort();
    };
  }, [
    activeSheetId,
    client,
    descriptor.id,
    descriptor.threadId,
    requestWindow?.columnCount,
    requestWindow?.columnStart,
    requestWindow?.rowCount,
    requestWindow?.rowStart,
    requestWindowCovered,
  ]);

  if (book.status === "loading")
    return <PreviewStatus icon="loading" title="Loading workbook" />;
  if (book.status === "error") {
    return (
      <PreviewStatus
        icon="error"
        title="Could not read workbook"
        detail={book.message}
      />
    );
  }
  if (!book.value.sheets.length || !activeSheet) {
    return (
      <PreviewStatus
        icon="empty"
        title="This workbook has no visible sheets."
      />
    );
  }

  return (
    <div className="spreadsheet-preview">
      <div
        className="spreadsheet-sheet-tabs"
        role="tablist"
        aria-label="Workbook sheets"
      >
        {book.value.sheets.map((sheet) => (
          <button
            key={sheet.id}
            className={sheet.id === activeSheet.id ? "active" : ""}
            type="button"
            role="tab"
            aria-selected={sheet.id === activeSheet.id}
            title={sheet.name}
            onClick={() => {
              if (sheet.id === activeSheet.id) return;
              clearRequestTimer(requestTimerRef);
              setActiveSheetId(sheet.id);
              setCachedRanges([]);
              setRequestedWindow(null);
              setRangeStatus({ status: "loading" });
              visibleRegionRef.current = null;
              rangeRequestStartedRef.current = false;
            }}
          >
            <span>{sheet.name}</span>
          </button>
        ))}
      </div>
      <div
        className="spreadsheet-glide-grid"
        aria-label={`${descriptor.title}, ${activeSheet.name}`}
        aria-busy={rangeStatus.status === "loading" && !requestWindowCovered}
      >
        <DataEditor
          key={activeSheet.id}
          columns={columns}
          rows={activeSheet.rowCount}
          getCellContent={getCellContent}
          rowMarkers={{ kind: "number", width: sheetRowHeaderWidth }}
          headerHeight={sheetColumnHeaderHeight}
          rowHeight={sheetRowHeight}
          theme={gridTheme}
          width="100%"
          height="100%"
          onVisibleRegionChanged={onVisibleRegionChanged}
        />
        {rangeStatus.status === "error" && (
          <div className="spreadsheet-error" role="alert">
            {rangeStatus.message}
          </div>
        )}
      </div>
    </div>
  );
}

function clearRequestTimer(timerRef: MutableRefObject<number | null>): void {
  if (timerRef.current !== null) {
    window.clearTimeout(timerRef.current);
    timerRef.current = null;
  }
}

function PreviewStatus({
  icon,
  title,
  detail,
}: {
  icon: "loading" | "error" | "empty";
  title: string;
  detail?: string;
}) {
  return (
    <div
      className="preview-status"
      role={icon === "error" ? "alert" : "status"}
    >
      {icon === "loading" ? (
        <Loader2 className="spin" size={22} />
      ) : icon === "error" ? (
        <AlertCircle size={22} />
      ) : (
        <FileQuestion size={22} />
      )}
      <strong>{title}</strong>
      {detail && <p>{detail}</p>}
    </div>
  );
}

function formatCell(cell: SpreadsheetPreviewCell | undefined): string {
  if (!cell) return "";
  if (cell.formatted != null) return cell.formatted;
  if (cell.value == null) return "";
  return String(cell.value);
}

function rangeWindow(range: SpreadsheetPreviewRange): SpreadsheetWindow {
  return {
    rowStart: range.rowStart,
    rowCount: range.rowCount,
    columnStart: range.columnStart,
    columnCount: range.columnCount,
  };
}

function rangeKey(range: SpreadsheetPreviewRange): string {
  return `${range.sheetId}:${range.rowStart}:${range.rowCount}:${range.columnStart}:${range.columnCount}`;
}

function sameSpreadsheetWindow(
  left: SpreadsheetWindow,
  right: SpreadsheetWindow,
): boolean {
  return (
    left.rowStart === right.rowStart &&
    left.rowCount === right.rowCount &&
    left.columnStart === right.columnStart &&
    left.columnCount === right.columnCount
  );
}

function columnLabel(index: number): string {
  let value = index + 1;
  let label = "";
  while (value > 0) {
    value -= 1;
    label = String.fromCharCode(65 + (value % 26)) + label;
    value = Math.floor(value / 26);
  }
  return label;
}

function errorMessage(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}

function isAbortError(cause: unknown): boolean {
  return cause instanceof Error && cause.name === "AbortError";
}

function buildGridTheme(): Theme {
  const base = getDefaultTheme();
  const css = (name: string, fallback: string) => {
    if (typeof document === "undefined") return fallback;
    return (
      getComputedStyle(document.documentElement)
        .getPropertyValue(name)
        .trim() || fallback
    );
  };
  const fontSize = css("--font-size-sm", base.baseFontStyle);
  const markerFontSize = css("--font-size-xs", base.markerFontStyle);
  const spacing = (name: string, fallback: number) => {
    const value = Number.parseFloat(css(name, `${fallback}px`));
    return Number.isFinite(value) ? value : fallback;
  };

  return {
    ...base,
    accentColor: css("--accent", base.accentColor),
    accentFg: css("--on-accent", base.accentFg),
    accentLight: css("--accent-subtle", base.accentLight),
    textDark: css("--text", base.textDark),
    textMedium: css("--text-secondary", base.textMedium),
    textLight: css("--text-muted", base.textLight),
    textBubble: css("--text", base.textBubble),
    bgIconHeader: css("--text-secondary", base.bgIconHeader),
    fgIconHeader: css("--on-accent", base.fgIconHeader),
    textHeader: css("--text", base.textHeader),
    textGroupHeader: css(
      "--text-secondary",
      base.textGroupHeader ?? base.textHeader,
    ),
    textHeaderSelected: css("--on-accent", base.textHeaderSelected),
    bgCell: css("--surface", base.bgCell),
    bgCellMedium: css("--surface-subtle", base.bgCellMedium),
    bgHeader: css("--surface-chrome", base.bgHeader),
    bgHeaderHasFocus: css("--accent-subtle", base.bgHeaderHasFocus),
    bgHeaderHovered: css("--surface-hover", base.bgHeaderHovered),
    bgBubble: css("--surface-active", base.bgBubble),
    bgBubbleSelected: css("--surface", base.bgBubbleSelected),
    bgSearchResult: css("--warning-subtle", base.bgSearchResult),
    borderColor: css("--border", base.borderColor),
    drilldownBorder: css("--border", base.drilldownBorder),
    linkColor: css("--accent", base.linkColor),
    cellHorizontalPadding: spacing("--space-3", base.cellHorizontalPadding),
    cellVerticalPadding: spacing("--space-2", base.cellVerticalPadding),
    headerFontStyle: `${base.headerFontStyle.split(" ")[0]} ${fontSize}`,
    baseFontStyle: fontSize,
    markerFontStyle: markerFontSize,
    fontFamily: css("--font-sans", base.fontFamily),
    editorFontSize: fontSize,
    lineHeight: Number.parseFloat(
      css("--line-height-body", String(base.lineHeight)),
    ),
    horizontalBorderColor: css("--border-subtle", base.borderColor),
    headerBottomBorderColor: css("--border", base.borderColor),
  };
}
