import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  AlertCircle,
  ExternalLink,
  FileQuestion,
  Loader2,
  Maximize2,
  Minus,
  Plus,
  RefreshCw,
  RotateCcw,
  Scan,
  Sheet,
  ZoomIn,
  ZoomOut,
} from "lucide-react";
import type { PDFDocumentProxy } from "pdfjs-dist";
import type { ApiClient } from "../api/client";
import { openPath } from "../platform";
import type {
  PreviewDescriptor,
  PreviewTarget,
  SpreadsheetPreview,
  SpreadsheetPreviewCell,
  SpreadsheetPreviewRange,
} from "../types";
import { detectLanguage, MonacoEditor } from "./MonacoEditor";

type LoadState<T> =
  | { status: "loading" }
  | { status: "ready"; value: T }
  | { status: "error"; message: string };

export function PreviewHost({
  client,
  threadId,
  workspaceRoot,
  target,
}: {
  client: ApiClient | null;
  threadId: string | null;
  workspaceRoot: string | null;
  target: PreviewTarget;
}) {
  const [reloadKey, setReloadKey] = useState(0);
  const [state, setState] = useState<LoadState<PreviewDescriptor>>({
    status: "loading",
  });

  useEffect(() => {
    let disposed = false;
    let resolved: PreviewDescriptor | null = null;
    if (!client || !threadId) {
      setState({
        status: "error",
        message: "Preview requires an active task.",
      });
      return;
    }

    setState({ status: "loading" });
    void client
      .resolvePreview(threadId, target)
      .then((descriptor) => {
        resolved = descriptor;
        if (!disposed) setState({ status: "ready", value: descriptor });
      })
      .catch((cause) => {
        if (!disposed)
          setState({ status: "error", message: errorMessage(cause) });
      });

    return () => {
      disposed = true;
      if (resolved) void client.closePreview(resolved.id);
    };
  }, [client, reloadKey, target, threadId]);

  if (state.status === "loading") {
    return <PreviewStatus icon="loading" title="Loading preview" />;
  }
  if (state.status === "error") {
    return (
      <PreviewStatus
        icon="error"
        title="Preview unavailable"
        detail={state.message}
        actionLabel="Try again"
        onAction={() => setReloadKey((current) => current + 1)}
      />
    );
  }

  if (!client) {
    return <PreviewStatus icon="error" title="Preview client is unavailable" />;
  }

  const descriptor = {
    ...state.value,
    externalPath:
      state.value.externalPath ??
      (target.type === "workspace" && workspaceRoot
        ? workspaceFilePath(workspaceRoot, target.path)
        : null),
  };
  return (
    <section
      className="preview-host"
      aria-label={`Preview ${descriptor.title}`}
    >
      <header className="preview-header">
        <div className="preview-heading">
          <strong title={descriptor.title}>{descriptor.title}</strong>
          <span title={descriptor.contentType}>{descriptor.contentType}</span>
          {descriptor.bytes != null && (
            <span>{formatBytes(descriptor.bytes)}</span>
          )}
          {descriptor.truncated && (
            <span className="preview-warning-pill">Truncated</span>
          )}
        </div>
        <div className="preview-header-actions">
          <button
            className="icon-button small"
            type="button"
            title="Reload preview"
            aria-label="Reload preview"
            onClick={() => setReloadKey((current) => current + 1)}
          >
            <RefreshCw size={14} />
          </button>
          {descriptor.externalPath && (
            <button
              className="icon-button small"
              type="button"
              title="Open with system application"
              aria-label="Open with system application"
              onClick={() => void openPath(descriptor.externalPath!)}
            >
              <ExternalLink size={14} />
            </button>
          )}
        </div>
      </header>
      <div className="preview-surface">
        <PreviewRenderer client={client} descriptor={descriptor} />
      </div>
    </section>
  );
}

function PreviewRenderer({
  client,
  descriptor,
}: {
  client: ApiClient;
  descriptor: PreviewDescriptor;
}) {
  switch (descriptor.renderer) {
    case "text":
    case "code":
      return <TextPreview client={client} descriptor={descriptor} />;
    case "image":
      return <ImagePreview client={client} descriptor={descriptor} />;
    case "pdf":
      return <PdfPreview client={client} descriptor={descriptor} />;
    case "spreadsheet":
      return <SpreadsheetGrid client={client} descriptor={descriptor} />;
    case "unsupported":
      return <UnsupportedPreview descriptor={descriptor} />;
    case "web":
      return (
        <PreviewStatus icon="empty" title="Open this URL in the browser tab." />
      );
  }
}

function TextPreview({
  client,
  descriptor,
}: {
  client: ApiClient;
  descriptor: PreviewDescriptor;
}) {
  const state = usePreviewBlob(client, descriptor);
  const [text, setText] = useState<LoadState<string>>({ status: "loading" });

  useEffect(() => {
    let disposed = false;
    if (state.status !== "ready") {
      setText(state.status === "error" ? state : { status: "loading" });
      return;
    }
    void state.value
      .text()
      .then((value) => {
        if (!disposed) setText({ status: "ready", value });
      })
      .catch((cause) => {
        if (!disposed)
          setText({ status: "error", message: errorMessage(cause) });
      });
    return () => {
      disposed = true;
    };
  }, [state]);

  if (text.status === "loading")
    return <PreviewStatus icon="loading" title="Loading file" />;
  if (text.status === "error") {
    return (
      <PreviewStatus
        icon="error"
        title="Could not read file"
        detail={text.message}
      />
    );
  }
  return (
    <div className="preview-code">
      <MonacoEditor
        value={text.value}
        language={detectLanguage(descriptor.title)}
        readOnly
        theme="vs"
      />
    </div>
  );
}

function ImagePreview({
  client,
  descriptor,
}: {
  client: ApiClient;
  descriptor: PreviewDescriptor;
}) {
  const state = usePreviewBlob(client, descriptor);
  const [mode, setMode] = useState<"fit" | "actual">("fit");
  const [zoom, setZoom] = useState(1);
  const [naturalSize, setNaturalSize] = useState({ width: 0, height: 0 });
  const objectUrl = useObjectUrl(state.status === "ready" ? state.value : null);

  if (state.status === "loading")
    return <PreviewStatus icon="loading" title="Loading image" />;
  if (state.status === "error") {
    return (
      <PreviewStatus
        icon="error"
        title="Could not load image"
        detail={state.message}
      />
    );
  }

  return (
    <div className="image-preview">
      <div
        className="preview-renderer-toolbar"
        role="toolbar"
        aria-label="Image controls"
      >
        <button
          className={`icon-button small ${mode === "fit" ? "active" : ""}`}
          type="button"
          title="Fit to view"
          aria-label="Fit image to view"
          aria-pressed={mode === "fit"}
          onClick={() => setMode("fit")}
        >
          <Maximize2 size={14} />
        </button>
        <button
          className={`icon-button small ${mode === "actual" ? "active" : ""}`}
          type="button"
          title="Actual size"
          aria-label="Show image at actual size"
          aria-pressed={mode === "actual"}
          onClick={() => {
            setMode("actual");
            setZoom(1);
          }}
        >
          <Scan size={14} />
        </button>
        <span className="preview-toolbar-divider" />
        <button
          className="icon-button small"
          type="button"
          title="Zoom out"
          aria-label="Zoom out"
          disabled={mode === "fit" || zoom <= 0.25}
          onClick={() => setZoom((current) => Math.max(0.25, current - 0.25))}
        >
          <ZoomOut size={14} />
        </button>
        <span className="preview-zoom-value">{Math.round(zoom * 100)}%</span>
        <button
          className="icon-button small"
          type="button"
          title="Zoom in"
          aria-label="Zoom in"
          disabled={mode === "fit" || zoom >= 4}
          onClick={() => setZoom((current) => Math.min(4, current + 0.25))}
        >
          <ZoomIn size={14} />
        </button>
        <button
          className="icon-button small"
          type="button"
          title="Reset zoom"
          aria-label="Reset zoom"
          disabled={mode === "fit" || zoom === 1}
          onClick={() => setZoom(1)}
        >
          <RotateCcw size={14} />
        </button>
      </div>
      <div className={`image-preview-canvas ${mode}`}>
        {objectUrl && (
          <img
            alt={descriptor.title}
            src={objectUrl}
            onLoad={(event) =>
              setNaturalSize({
                width: event.currentTarget.naturalWidth,
                height: event.currentTarget.naturalHeight,
              })
            }
            style={
              mode === "actual" && naturalSize.width
                ? {
                    width: `${naturalSize.width * zoom}px`,
                    height: `${naturalSize.height * zoom}px`,
                  }
                : undefined
            }
          />
        )}
      </div>
    </div>
  );
}

function PdfPreview({
  client,
  descriptor,
}: {
  client: ApiClient;
  descriptor: PreviewDescriptor;
}) {
  const state = usePreviewBlob(client, descriptor);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [document, setDocument] = useState<PDFDocumentProxy | null>(null);
  const [pageNumber, setPageNumber] = useState(1);
  const [scale, setScale] = useState(1.15);
  const [renderError, setRenderError] = useState<string | null>(null);
  const [rendering, setRendering] = useState(false);

  useEffect(() => {
    let disposed = false;
    let loaded: PDFDocumentProxy | null = null;
    if (state.status !== "ready") {
      setDocument(null);
      return;
    }
    setRenderError(null);
    setPageNumber(1);
    void (async () => {
      const pdfjs = await import("pdfjs-dist");
      const worker = await import("pdfjs-dist/build/pdf.worker.min.mjs?url");
      pdfjs.GlobalWorkerOptions.workerSrc = worker.default;
      const bytes = new Uint8Array(await state.value.arrayBuffer());
      loaded = await pdfjs.getDocument({ data: bytes }).promise;
      if (!disposed) setDocument(loaded);
    })().catch((cause) => {
      if (!disposed) setRenderError(errorMessage(cause));
    });
    return () => {
      disposed = true;
      void loaded?.destroy();
    };
  }, [state]);

  useEffect(() => {
    let cancelled = false;
    let renderTask: { cancel(): void; promise: Promise<void> } | null = null;
    if (!document || !canvasRef.current) return;
    setRendering(true);
    setRenderError(null);
    void document
      .getPage(pageNumber)
      .then((page) => {
        if (cancelled || !canvasRef.current) return;
        const viewport = page.getViewport({ scale });
        const canvas = canvasRef.current;
        const context = canvas.getContext("2d");
        if (!context) throw new Error("Canvas rendering is unavailable.");
        const pixelRatio = Math.min(window.devicePixelRatio || 1, 2);
        canvas.width = Math.floor(viewport.width * pixelRatio);
        canvas.height = Math.floor(viewport.height * pixelRatio);
        canvas.style.width = `${Math.floor(viewport.width)}px`;
        canvas.style.height = `${Math.floor(viewport.height)}px`;
        renderTask = page.render({
          canvasContext: context,
          viewport,
          transform:
            pixelRatio === 1 ? undefined : [pixelRatio, 0, 0, pixelRatio, 0, 0],
        });
        return renderTask.promise;
      })
      .then(() => {
        if (!cancelled) setRendering(false);
      })
      .catch((cause) => {
        if (!cancelled && errorMessage(cause) !== "Rendering cancelled") {
          setRenderError(errorMessage(cause));
          setRendering(false);
        }
      });
    return () => {
      cancelled = true;
      renderTask?.cancel();
    };
  }, [document, pageNumber, scale]);

  if (
    state.status === "loading" ||
    (state.status === "ready" && !document && !renderError)
  ) {
    return <PreviewStatus icon="loading" title="Loading PDF" />;
  }
  if (state.status === "error" || renderError) {
    return (
      <PreviewStatus
        icon="error"
        title="Could not render PDF"
        detail={
          state.status === "error" ? state.message : (renderError ?? undefined)
        }
      />
    );
  }

  return (
    <div className="pdf-preview">
      <div
        className="preview-renderer-toolbar"
        role="toolbar"
        aria-label="PDF controls"
      >
        <button
          className="icon-button small"
          type="button"
          title="Previous page"
          aria-label="Previous PDF page"
          disabled={pageNumber <= 1}
          onClick={() => setPageNumber((current) => Math.max(1, current - 1))}
        >
          <Minus size={14} />
        </button>
        <label className="pdf-page-control">
          <span className="sr-only">PDF page number</span>
          <input
            type="number"
            min={1}
            max={document?.numPages ?? 1}
            value={pageNumber}
            onChange={(event) =>
              setPageNumber(
                Math.min(
                  document?.numPages ?? 1,
                  Math.max(1, Number(event.target.value) || 1),
                ),
              )
            }
          />
          <span>/ {document?.numPages ?? 1}</span>
        </label>
        <button
          className="icon-button small"
          type="button"
          title="Next page"
          aria-label="Next PDF page"
          disabled={pageNumber >= (document?.numPages ?? 1)}
          onClick={() =>
            setPageNumber((current) =>
              Math.min(document?.numPages ?? 1, current + 1),
            )
          }
        >
          <Plus size={14} />
        </button>
        <span className="preview-toolbar-divider" />
        <button
          className="icon-button small"
          type="button"
          title="Zoom out"
          aria-label="Zoom PDF out"
          disabled={scale <= 0.5}
          onClick={() => setScale((current) => Math.max(0.5, current - 0.15))}
        >
          <ZoomOut size={14} />
        </button>
        <span className="preview-zoom-value">{Math.round(scale * 100)}%</span>
        <button
          className="icon-button small"
          type="button"
          title="Zoom in"
          aria-label="Zoom PDF in"
          disabled={scale >= 3}
          onClick={() => setScale((current) => Math.min(3, current + 0.15))}
        >
          <ZoomIn size={14} />
        </button>
        {rendering && (
          <Loader2 className="spin preview-toolbar-loader" size={13} />
        )}
      </div>
      <div className="pdf-preview-canvas">
        <canvas
          ref={canvasRef}
          aria-label={`Page ${pageNumber} of ${document?.numPages ?? 1}`}
        />
      </div>
    </div>
  );
}

const sheetRowHeight = 25;
const sheetColumnWidth = 120;
const sheetRowHeaderWidth = 48;
const sheetColumnHeaderHeight = 27;
const sheetChunkRows = 100;
const sheetChunkColumns = 20;

function SpreadsheetGrid({
  client,
  descriptor,
}: {
  client: ApiClient;
  descriptor: PreviewDescriptor;
}) {
  const viewportRef = useRef<HTMLDivElement>(null);
  const [book, setBook] = useState<LoadState<SpreadsheetPreview>>({
    status: "loading",
  });
  const [activeSheetId, setActiveSheetId] = useState<string | null>(null);
  const [range, setRange] = useState<LoadState<SpreadsheetPreviewRange>>({
    status: "loading",
  });
  const [windowStart, setWindowStart] = useState({ row: 0, column: 0 });

  useEffect(() => {
    let disposed = false;
    setBook({ status: "loading" });
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
    };
  }, [client, descriptor.id]);

  useEffect(() => {
    let disposed = false;
    if (!activeSheetId) return;
    setRange({ status: "loading" });
    void client
      .getSpreadsheetPreviewRange(
        descriptor.threadId,
        descriptor.id,
        activeSheetId,
        {
          rowStart: windowStart.row,
          rowCount: sheetChunkRows,
          columnStart: windowStart.column,
          columnCount: sheetChunkColumns,
        },
      )
      .then((value) => {
        if (!disposed) setRange({ status: "ready", value });
      })
      .catch((cause) => {
        if (!disposed)
          setRange({ status: "error", message: errorMessage(cause) });
      });
    return () => {
      disposed = true;
    };
  }, [
    activeSheetId,
    client,
    descriptor.id,
    windowStart.column,
    windowStart.row,
  ]);

  const activeSheet =
    book.status === "ready"
      ? (book.value.sheets.find((sheet) => sheet.id === activeSheetId) ?? null)
      : null;
  const cells = useMemo(() => {
    const next = new Map<string, SpreadsheetPreviewCell>();
    if (range.status === "ready") {
      for (const cell of range.value.cells)
        next.set(`${cell.row}:${cell.column}`, cell);
    }
    return next;
  }, [range]);

  const updateVisibleWindow = useCallback((element: HTMLDivElement) => {
    const row = Math.max(
      0,
      Math.floor(
        (element.scrollTop - sheetColumnHeaderHeight) / sheetRowHeight,
      ),
    );
    const column = Math.max(
      0,
      Math.floor((element.scrollLeft - sheetRowHeaderWidth) / sheetColumnWidth),
    );
    const nextRow = Math.floor(row / sheetChunkRows) * sheetChunkRows;
    const nextColumn =
      Math.floor(column / sheetChunkColumns) * sheetChunkColumns;
    setWindowStart((current) => {
      if (current.row === nextRow && current.column === nextColumn)
        return current;
      return { row: nextRow, column: nextColumn };
    });
  }, []);

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

  const firstRow =
    range.status === "ready" ? range.value.rowStart : windowStart.row;
  const firstColumn =
    range.status === "ready" ? range.value.columnStart : windowStart.column;
  const rowCount = Math.min(
    sheetChunkRows,
    Math.max(0, activeSheet.rowCount - firstRow),
  );
  const columnCount = Math.min(
    sheetChunkColumns,
    Math.max(0, activeSheet.columnCount - firstColumn),
  );

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
              setActiveSheetId(sheet.id);
              setWindowStart({ row: 0, column: 0 });
              viewportRef.current?.scrollTo(0, 0);
            }}
          >
            <Sheet size={13} />
            <span>{sheet.name}</span>
          </button>
        ))}
      </div>
      <div
        ref={viewportRef}
        className="spreadsheet-viewport"
        role="grid"
        aria-label={`${descriptor.title}, ${activeSheet.name}`}
        aria-rowcount={activeSheet.rowCount}
        aria-colcount={activeSheet.columnCount}
        tabIndex={0}
        onScroll={(event) => updateVisibleWindow(event.currentTarget)}
        onKeyDown={(event) => {
          const step = event.shiftKey ? 5 : 1;
          if (event.key === "ArrowDown")
            event.currentTarget.scrollBy(0, sheetRowHeight * step);
          else if (event.key === "ArrowUp")
            event.currentTarget.scrollBy(0, -sheetRowHeight * step);
          else if (event.key === "ArrowRight")
            event.currentTarget.scrollBy(sheetColumnWidth * step, 0);
          else if (event.key === "ArrowLeft")
            event.currentTarget.scrollBy(-sheetColumnWidth * step, 0);
          else return;
          event.preventDefault();
        }}
      >
        <div
          className="spreadsheet-canvas"
          style={{
            width: `${sheetRowHeaderWidth + Math.max(1, activeSheet.columnCount) * sheetColumnWidth}px`,
            height: `${sheetColumnHeaderHeight + Math.max(1, activeSheet.rowCount) * sheetRowHeight}px`,
          }}
        >
          <div
            role="presentation"
            style={{
              position: "sticky",
              zIndex: 3,
              top: 0,
              width: "100%",
              height: `${sheetColumnHeaderHeight}px`,
            }}
          >
            <div
              className="spreadsheet-corner"
              role="presentation"
              style={{
                position: "sticky",
                left: 0,
              }}
            />
            {Array.from(
              { length: columnCount },
              (_, offset) => firstColumn + offset,
            ).map((column) => (
              <div
                className="spreadsheet-column-header"
                key={`column-${column}`}
                role="columnheader"
                style={{
                  left: `${sheetRowHeaderWidth + column * sheetColumnWidth}px`,
                }}
              >
                {columnLabel(column)}
              </div>
            ))}
          </div>
          <div
            role="presentation"
            style={{
              position: "sticky",
              zIndex: 2,
              left: 0,
              width: `${sheetRowHeaderWidth}px`,
              height: `${Math.max(1, activeSheet.rowCount) * sheetRowHeight}px`,
            }}
          >
            {Array.from(
              { length: rowCount },
              (_, offset) => firstRow + offset,
            ).map((row) => (
              <div
                className="spreadsheet-row-header"
                key={`row-${row}`}
                role="rowheader"
                style={{ top: `${row * sheetRowHeight}px` }}
              >
                {row + 1}
              </div>
            ))}
          </div>
          {Array.from(
            { length: rowCount },
            (_, rowOffset) => firstRow + rowOffset,
          ).flatMap((row) =>
            Array.from(
              { length: columnCount },
              (_, columnOffset) => firstColumn + columnOffset,
            ).map((column) => {
              const cell = cells.get(`${row}:${column}`);
              const text = formatCell(cell);
              return (
                <div
                  className="spreadsheet-cell"
                  key={`${row}:${column}`}
                  role="gridcell"
                  aria-rowindex={row + 1}
                  aria-colindex={column + 1}
                  title={cell?.formula ? `${cell.formula}\n${text}` : text}
                  style={{
                    left: `${sheetRowHeaderWidth + column * sheetColumnWidth}px`,
                    top: `${sheetColumnHeaderHeight + row * sheetRowHeight}px`,
                  }}
                >
                  {text}
                </div>
              );
            }),
          )}
        </div>
        {range.status === "loading" && (
          <div className="spreadsheet-loading" aria-live="polite">
            <Loader2 className="spin" size={14} /> Loading cells
          </div>
        )}
        {range.status === "error" && (
          <div className="spreadsheet-error" role="alert">
            {range.message}
          </div>
        )}
      </div>
    </div>
  );
}

function UnsupportedPreview({ descriptor }: { descriptor: PreviewDescriptor }) {
  return (
    <div className="unsupported-preview">
      <FileQuestion size={28} />
      <h2>No built-in preview</h2>
      <p>{descriptor.contentType}</p>
      {descriptor.externalPath && (
        <button
          className="secondary-button compact"
          type="button"
          onClick={() => void openPath(descriptor.externalPath!)}
        >
          <ExternalLink size={14} />
          Open with system application
        </button>
      )}
    </div>
  );
}

function PreviewStatus({
  icon,
  title,
  detail,
  actionLabel,
  onAction,
}: {
  icon: "loading" | "error" | "empty";
  title: string;
  detail?: string;
  actionLabel?: string;
  onAction?: () => void;
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
      {actionLabel && onAction && (
        <button
          className="secondary-button compact"
          type="button"
          onClick={onAction}
        >
          {actionLabel}
        </button>
      )}
    </div>
  );
}

function usePreviewBlob(
  client: ApiClient,
  descriptor: PreviewDescriptor,
): LoadState<Blob> {
  const [state, setState] = useState<LoadState<Blob>>({ status: "loading" });
  useEffect(() => {
    let disposed = false;
    setState({ status: "loading" });
    void client
      .getPreviewContent(descriptor.threadId, descriptor.id)
      .then((value) => {
        if (!disposed) setState({ status: "ready", value });
      })
      .catch((cause) => {
        if (!disposed)
          setState({ status: "error", message: errorMessage(cause) });
      });
    return () => {
      disposed = true;
    };
  }, [client, descriptor.id, descriptor.revision]);
  return state;
}

function useObjectUrl(blob: Blob | null): string | null {
  const [url, setUrl] = useState<string | null>(null);
  useEffect(() => {
    if (!blob) {
      setUrl(null);
      return;
    }
    const next = URL.createObjectURL(blob);
    setUrl(next);
    return () => URL.revokeObjectURL(next);
  }, [blob]);
  return url;
}

function formatCell(cell: SpreadsheetPreviewCell | undefined): string {
  if (!cell) return "";
  if (cell.formatted != null) return cell.formatted;
  if (cell.value == null) return "";
  return String(cell.value);
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

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function workspaceFilePath(root: string, relativePath: string): string {
  const separator = root.includes("\\") ? "\\" : "/";
  const base = root.replace(/[\\/]+$/, "");
  const relative = relativePath
    .replace(/^[\\/]+/, "")
    .replace(/[\\/]+/g, separator);
  return `${base}${separator}${relative}`;
}

function errorMessage(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}
