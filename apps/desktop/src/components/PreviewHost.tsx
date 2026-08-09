import { lazy, Suspense, useEffect, useMemo, useRef, useState } from "react";
import {
  AlertCircle,
  ExternalLink,
  FileQuestion,
  Loader2,
  Minus,
  Plus,
  RefreshCw,
  ZoomIn,
  ZoomOut,
} from "lucide-react";
import type { PDFDocumentProxy } from "pdfjs-dist";
import type { ApiClient } from "../api/client";
import { openPath } from "../platform";
import type {
  InlineImageAttachment,
  PreviewDescriptor,
  PreviewTarget,
} from "../types";
import { detectLanguage, MonacoEditor } from "./MonacoEditor";
import { MarkdownContent } from "./MarkdownContent";

const GlideSpreadsheetGrid = lazy(() =>
  import("./SpreadsheetGrid").then(({ SpreadsheetGrid }) => ({
    default: SpreadsheetGrid,
  })),
);

type LoadState<T> =
  | { status: "loading" }
  | { status: "ready"; value: T }
  | { status: "error"; message: string };

export type ImagePreviewSource = Pick<
  InlineImageAttachment,
  "contentType" | "data" | "name"
> & {
  content_type?: string;
};

export function PreviewHost({
  client,
  threadId,
  workspaceRoot,
  target,
  onOpenMarkdownLink,
}: {
  client: ApiClient | null;
  threadId: string | null;
  workspaceRoot: string | null;
  target: PreviewTarget;
  onOpenMarkdownLink?(href: string, baseWorkspacePath?: string | null): void;
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
        <PreviewRenderer
          client={client}
          descriptor={descriptor}
          onOpenMarkdownLink={onOpenMarkdownLink}
        />
      </div>
    </section>
  );
}

function PreviewRenderer({
  client,
  descriptor,
  onOpenMarkdownLink,
}: {
  client: ApiClient;
  descriptor: PreviewDescriptor;
  onOpenMarkdownLink?(href: string, baseWorkspacePath?: string | null): void;
}) {
  switch (descriptor.renderer) {
    case "text":
    case "code":
      return (
        <TextPreview
          client={client}
          descriptor={descriptor}
          onOpenMarkdownLink={onOpenMarkdownLink}
        />
      );
    case "image":
      return <ImagePreview client={client} descriptor={descriptor} />;
    case "pdf":
      return <PdfPreview client={client} descriptor={descriptor} />;
    case "spreadsheet":
      return (
        <Suspense
          fallback={<PreviewStatus icon="loading" title="Loading workbook" />}
        >
          <GlideSpreadsheetGrid client={client} descriptor={descriptor} />
        </Suspense>
      );
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
  onOpenMarkdownLink,
}: {
  client: ApiClient;
  descriptor: PreviewDescriptor;
  onOpenMarkdownLink?(href: string, baseWorkspacePath?: string | null): void;
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
  if (isMarkdownPreview(descriptor)) {
    const baseWorkspacePath =
      descriptor.target.type === "workspace" ? descriptor.target.path : null;
    return (
      <MarkdownContent
        baseWorkspacePath={baseWorkspacePath}
        className="preview-markdown"
        onOpenLink={(href) => onOpenMarkdownLink?.(href, baseWorkspacePath)}
        text={text.value}
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

function isMarkdownPreview(descriptor: PreviewDescriptor): boolean {
  return (
    descriptor.contentType.toLowerCase().includes("markdown") ||
    /\.md(?:own)?$/i.test(descriptor.title)
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

  return <ImagePreviewSurface title={descriptor.title} objectUrl={objectUrl} />;
}

export function InlineImagePreview({ image }: { image: ImagePreviewSource }) {
  const blob = useMemo(
    () =>
      new Blob([new Uint8Array(image.data)], {
        type:
          image.contentType || image.content_type || "application/octet-stream",
      }),
    [image],
  );
  const objectUrl = useObjectUrl(blob);

  return (
    <ImagePreviewSurface
      title={image.name?.trim() || "图片"}
      objectUrl={objectUrl}
    />
  );
}

function ImagePreviewSurface({
  title,
  objectUrl,
}: {
  title: string;
  objectUrl: string | null;
}) {
  const [zoom, setZoom] = useState<number | "fit">("fit");
  const [naturalSize, setNaturalSize] = useState({ width: 0, height: 0 });
  const isFit = zoom === "fit";

  return (
    <div className="image-preview">
      <div className="image-preview-toolbar">
        <select
          className="image-preview-zoom-select"
          aria-label="图片缩放"
          value={zoom}
          onChange={(event) =>
            setZoom(
              event.target.value === "fit" ? "fit" : Number(event.target.value),
            )
          }
        >
          <option value={0.25}>25%</option>
          <option value={0.5}>50%</option>
          <option value={1}>100%</option>
          <option value={1.5}>150%</option>
          <option value={2}>200%</option>
          <option value="fit">适应窗口</option>
        </select>
      </div>
      <div className={`image-preview-canvas ${isFit ? "fit" : "actual"}`}>
        {objectUrl && (
          <img
            alt={title}
            src={objectUrl}
            onLoad={(event) =>
              setNaturalSize({
                width: event.currentTarget.naturalWidth,
                height: event.currentTarget.naturalHeight,
              })
            }
            style={
              !isFit && naturalSize.width
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
