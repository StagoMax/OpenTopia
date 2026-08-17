import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  AlertCircle,
  ExternalLink,
  FileQuestion,
  Loader2,
  Minus,
  Plus,
  RefreshCw,
  RotateCcw,
  Save,
  ZoomIn,
  ZoomOut,
} from "lucide-react";
import type { PDFDocumentProxy } from "pdfjs-dist";
import { ApiResponseError, type ApiClient } from "../api/client";
import { resolveMarkdownLink } from "../markdownLinks";
import { openPath } from "../platform";
import type {
  InlineImageAttachment,
  PreviewDescriptor,
  PreviewTarget,
} from "../types";
import { FileTypeIcon } from "./FileTypeIcon";
import { detectLanguage, MonacoEditor } from "./MonacoEditor";
import { MarkdownContent } from "./MarkdownContent";
import { Badge, Button, SegmentedControl } from "./ui";

const GlideSpreadsheetGrid = lazy(() =>
  import("./SpreadsheetGrid").then(({ SpreadsheetGrid }) => ({
    default: SpreadsheetGrid,
  })),
);

const DocxPreview = lazy(() =>
  import("./DocxPreview").then(({ DocxPreview }) => ({
    default: DocxPreview,
  })),
);

type LoadState<T> =
  | { status: "loading" }
  | { status: "ready"; value: T }
  | { status: "error"; message: string };

export type PreviewViewMode = "preview" | "source" | "split";

export type PreviewDocumentSession = {
  mode: PreviewViewMode;
  draft: string;
  baseline: string;
  revision: string;
  dirty: boolean;
  externalChanged: boolean;
};

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
  sessionId,
  sessionState,
  onSessionChange,
}: {
  client: ApiClient | null;
  threadId: string | null;
  workspaceRoot: string | null;
  target: PreviewTarget;
  onOpenMarkdownLink?(href: string, baseWorkspacePath?: string | null): void;
  sessionId?: string;
  sessionState?: PreviewDocumentSession;
  onSessionChange?(sessionId: string, state: PreviewDocumentSession): void;
}) {
  const [reloadKey, setReloadKey] = useState(0);
  const [state, setState] = useState<LoadState<PreviewDescriptor>>({
    status: "loading",
  });
  const [dirty, setDirty] = useState(sessionState?.dirty ?? false);

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
      if (resolved) {
        void client
          .closePreview(resolved.threadId, resolved.id)
          .catch(() => undefined);
      }
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
          {target.type === "attachment" ? (
            <FileTypeIcon
              name={descriptor.title}
              contentType={descriptor.contentType}
              size={18}
            />
          ) : null}
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
            onClick={() => {
              if (
                dirty &&
                !window.confirm("重新加载会丢弃尚未保存的 Markdown 更改，是否继续？")
              ) {
                return;
              }
              setReloadKey((current) => current + 1);
            }}
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
          sessionId={sessionId}
          sessionState={sessionState}
          onSessionChange={onSessionChange}
          onDirtyChange={setDirty}
        />
      </div>
    </section>
  );
}

function PreviewRenderer({
  client,
  descriptor,
  onOpenMarkdownLink,
  sessionId,
  sessionState,
  onSessionChange,
  onDirtyChange,
}: {
  client: ApiClient;
  descriptor: PreviewDescriptor;
  onOpenMarkdownLink?(href: string, baseWorkspacePath?: string | null): void;
  sessionId?: string;
  sessionState?: PreviewDocumentSession;
  onSessionChange?(sessionId: string, state: PreviewDocumentSession): void;
  onDirtyChange?(dirty: boolean): void;
}) {
  switch (descriptor.renderer) {
    case "text":
    case "code":
      return (
        <TextPreview
          client={client}
          descriptor={descriptor}
          onOpenMarkdownLink={onOpenMarkdownLink}
          sessionId={sessionId}
          sessionState={sessionState}
          onSessionChange={onSessionChange}
          onDirtyChange={onDirtyChange}
        />
      );
    case "image":
      return <ImagePreview client={client} descriptor={descriptor} />;
    case "pdf":
      return <PdfPreview client={client} descriptor={descriptor} />;
    case "document":
      return (
        <Suspense
          fallback={<PreviewStatus icon="loading" title="Loading document" />}
        >
          <DocxPreview client={client} descriptor={descriptor} />
        </Suspense>
      );
    case "spreadsheet":
      return (
        <Suspense
          fallback={<PreviewStatus icon="loading" title="Loading workbook" />}
        >
          <GlideSpreadsheetGrid client={client} descriptor={descriptor} />
        </Suspense>
      );
    case "unsupported":
      return descriptor.handlerId ? (
        <PluginPreview
          client={client}
          descriptor={descriptor}
          onOpenMarkdownLink={onOpenMarkdownLink}
        />
      ) : (
        <UnsupportedPreview descriptor={descriptor} />
      );
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
  sessionId,
  sessionState,
  onSessionChange,
  onDirtyChange,
}: {
  client: ApiClient;
  descriptor: PreviewDescriptor;
  onOpenMarkdownLink?(href: string, baseWorkspacePath?: string | null): void;
  sessionId?: string;
  sessionState?: PreviewDocumentSession;
  onSessionChange?(sessionId: string, state: PreviewDocumentSession): void;
  onDirtyChange?(dirty: boolean): void;
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
    const baseResourcePath =
      descriptor.target.type === "workspace" || descriptor.target.type === "local"
        ? descriptor.target.path
        : descriptor.externalPath;
    return (
      <MarkdownDocumentView
        baseResourcePath={baseResourcePath ?? null}
        client={client}
        descriptor={descriptor}
        loadedText={text.value}
        onDirtyChange={onDirtyChange}
        onOpenMarkdownLink={onOpenMarkdownLink}
        onSessionChange={onSessionChange}
        sessionId={sessionId}
        sessionState={sessionState}
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

const markdownViewModes: ReadonlyArray<{
  value: PreviewViewMode;
  label: string;
}> = [
  { value: "preview", label: "预览" },
  { value: "source", label: "源码" },
  { value: "split", label: "分屏" },
];

function MarkdownDocumentView({
  baseResourcePath,
  client,
  descriptor,
  loadedText,
  onDirtyChange,
  onOpenMarkdownLink,
  onSessionChange,
  sessionId,
  sessionState,
}: {
  baseResourcePath: string | null;
  client: ApiClient;
  descriptor: PreviewDescriptor;
  loadedText: string;
  onDirtyChange?(dirty: boolean): void;
  onOpenMarkdownLink?(href: string, baseResourcePath?: string | null): void;
  onSessionChange?(sessionId: string, state: PreviewDocumentSession): void;
  sessionId?: string;
  sessionState?: PreviewDocumentSession;
}) {
  const restoredDraft = Boolean(sessionState?.dirty);
  const [mode, setMode] = useState<PreviewViewMode>(
    sessionState?.mode ?? "preview",
  );
  const [draft, setDraft] = useState(
    restoredDraft ? sessionState!.draft : loadedText,
  );
  const [baseline, setBaseline] = useState(
    restoredDraft ? sessionState!.baseline : loadedText,
  );
  const [revision, setRevision] = useState(
    restoredDraft ? sessionState!.revision : descriptor.revision,
  );
  const [externalChanged, setExternalChanged] = useState(
    Boolean(
      sessionState?.externalChanged ||
        (restoredDraft && sessionState?.revision !== descriptor.revision),
    ),
  );
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const currentRef = useRef({ draft, baseline, revision });
  const dirty = draft !== baseline;

  const resolveImage = useResourceImageResolver(
    client,
    descriptor.threadId,
    baseResourcePath,
  );

  useEffect(() => {
    currentRef.current = { draft, baseline, revision };
  }, [baseline, draft, revision]);

  useEffect(() => {
    onDirtyChange?.(dirty);
  }, [dirty, onDirtyChange]);

  useEffect(() => {
    if (!sessionId) return;
    onSessionChange?.(sessionId, {
      mode,
      draft,
      baseline,
      revision,
      dirty,
      externalChanged,
    });
  }, [
    baseline,
    dirty,
    draft,
    externalChanged,
    mode,
    onSessionChange,
    revision,
    sessionId,
  ]);

  async function readLatest(): Promise<{
    text: string;
    descriptor: PreviewDescriptor;
  }> {
    const latestDescriptor = await client.getResourceMetadata(descriptor);
    const blob = await client.getPreviewContent(descriptor.threadId, descriptor.id);
    return { text: await blob.text(), descriptor: latestDescriptor };
  }

  async function reloadFromDisk(confirmDiscard: boolean) {
    if (
      confirmDiscard &&
      dirty &&
      !window.confirm("重新载入会丢弃尚未保存的 Markdown 更改，是否继续？")
    ) {
      return;
    }
    setMessage(null);
    try {
      const latest = await readLatest();
      setDraft(latest.text);
      setBaseline(latest.text);
      setRevision(latest.descriptor.revision);
      setExternalChanged(false);
      setSaved(false);
    } catch (cause) {
      setMessage(errorMessage(cause));
    }
  }

  async function saveDocument() {
    if (!dirty || saving || !descriptor.capabilities.write) return;
    setSaving(true);
    setSaved(false);
    setMessage(null);
    try {
      const updated = await client.writeResourceContent(
        descriptor,
        draft,
        revision,
      );
      setBaseline(draft);
      setRevision(updated.revision);
      setExternalChanged(false);
      setSaved(true);
    } catch (cause) {
      if (cause instanceof ApiResponseError && cause.status === 409) {
        setExternalChanged(true);
        setMessage("文件已被其他程序修改。请重新载入后再保存。当前草稿不会丢失。");
      } else {
        setMessage(errorMessage(cause));
      }
    } finally {
      setSaving(false);
    }
  }

  useEffect(() => {
    if (!descriptor.capabilities.watch) return;
    let disposed = false;
    let checking = false;
    const check = async () => {
      if (checking) return;
      checking = true;
      try {
        const latest = await client.getResourceMetadata(descriptor);
        if (disposed || latest.revision === currentRef.current.revision) return;
        if (currentRef.current.draft !== currentRef.current.baseline) {
          setExternalChanged(true);
          return;
        }
        const blob = await client.getPreviewContent(
          descriptor.threadId,
          descriptor.id,
        );
        const text = await blob.text();
        if (disposed) return;
        setDraft(text);
        setBaseline(text);
        setRevision(latest.revision);
        setExternalChanged(false);
        setSaved(false);
      } catch {
        // A transient metadata failure should not interrupt editing. Explicit
        // save/reload actions surface actionable errors.
      } finally {
        checking = false;
      }
    };
    const timer = window.setInterval(() => void check(), 2_500);
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, [client, descriptor]);

  useEffect(() => {
    const handleSaveShortcut = (event: KeyboardEvent) => {
      if (!(event.ctrlKey || event.metaKey) || event.key.toLowerCase() !== "s") {
        return;
      }
      event.preventDefault();
      void saveDocument();
    };
    window.addEventListener("keydown", handleSaveShortcut);
    return () => window.removeEventListener("keydown", handleSaveShortcut);
  });

  const showSource = mode === "source" || mode === "split";
  const showPreview = mode === "preview" || mode === "split";
  return (
    <div className="markdown-document-view">
      <div
        className="markdown-document-toolbar"
        role="toolbar"
        aria-label="Markdown 文档操作"
      >
        <SegmentedControl
          label="Markdown 显示模式"
          onChange={(value: PreviewViewMode) => setMode(value)}
          options={markdownViewModes}
          value={mode}
        />
        <div className="markdown-document-status" aria-live="polite">
          {externalChanged ? (
            <Badge variant="danger">磁盘内容已变化</Badge>
          ) : dirty ? (
            <Badge variant="warning">未保存</Badge>
          ) : saved ? (
            <Badge variant="success">已保存</Badge>
          ) : descriptor.readonly ? (
            <Badge>只读</Badge>
          ) : null}
          {externalChanged ? (
            <Button
              size="compact"
              variant="quiet"
              onClick={() => void reloadFromDisk(true)}
            >
              <RotateCcw size={14} aria-hidden="true" />
              重新载入
            </Button>
          ) : null}
          {descriptor.capabilities.write ? (
            <Button
              size="compact"
              variant="primary"
              disabled={!dirty || saving || externalChanged}
              onClick={() => void saveDocument()}
            >
              {saving ? (
                <Loader2 className="spin" size={14} aria-hidden="true" />
              ) : (
                <Save size={14} aria-hidden="true" />
              )}
              {saving ? "保存中" : "保存"}
            </Button>
          ) : null}
        </div>
      </div>
      {message ? (
        <div className="markdown-document-message" role="alert">
          <AlertCircle size={14} aria-hidden="true" />
          <span>{message}</span>
        </div>
      ) : null}
      <div className={`markdown-document-body mode-${mode}`}>
        {showSource ? (
          <section className="markdown-source-pane" aria-label="Markdown 源码">
            <MonacoEditor
              language="markdown"
              onChange={(value) => {
                setDraft(value);
                setSaved(false);
                setMessage(null);
              }}
              readOnly={!descriptor.capabilities.write}
              theme="vs"
              value={draft}
            />
          </section>
        ) : null}
        {showPreview ? (
          <section className="markdown-preview-pane" aria-label="Markdown 预览">
            <MarkdownContent
              baseResourcePath={baseResourcePath}
              className="preview-markdown"
              onOpenLink={(href) =>
                onOpenMarkdownLink?.(href, baseResourcePath)
              }
              onResolveImage={resolveImage}
              text={draft}
            />
          </section>
        ) : null}
      </div>
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

function PluginPreview({
  client,
  descriptor,
  onOpenMarkdownLink,
}: {
  client: ApiClient;
  descriptor: PreviewDescriptor;
  onOpenMarkdownLink?(href: string, baseResourcePath?: string | null): void;
}) {
  const [state, setState] = useState<LoadState<unknown>>({ status: "loading" });

  useEffect(() => {
    let disposed = false;
    setState({ status: "loading" });
    void client
      .invokeMediaHandler(descriptor.threadId, {
        operation: "preview",
        contributionId: descriptor.handlerId ?? undefined,
        resourceId: descriptor.id,
        contentType: descriptor.contentType,
      })
      .then((response) => {
        if (!disposed)
          setState({ status: "ready", value: response.output.payload });
      })
      .catch((cause) => {
        if (!disposed) {
          setState({ status: "error", message: errorMessage(cause) });
        }
      });
    return () => {
      disposed = true;
    };
  }, [
    client,
    descriptor.contentType,
    descriptor.handlerId,
    descriptor.id,
    descriptor.threadId,
  ]);

  if (state.status === "loading") {
    return <PreviewStatus icon="loading" title="Loading plugin preview" />;
  }
  if (state.status === "error") {
    return (
      <PreviewStatus
        icon="error"
        title="Plugin preview failed"
        detail={state.message}
      />
    );
  }
  return (
    <PluginPreviewPayload
      client={client}
      descriptor={descriptor}
      onOpenMarkdownLink={onOpenMarkdownLink}
      payload={state.value}
    />
  );
}

function PluginPreviewPayload({
  client,
  descriptor,
  onOpenMarkdownLink,
  payload,
}: {
  client: ApiClient;
  descriptor: PreviewDescriptor;
  onOpenMarkdownLink?(href: string, baseResourcePath?: string | null): void;
  payload: unknown;
}) {
  const record = isRecord(payload) ? payload : null;
  const type = typeof record?.type === "string" ? record.type : null;
  const text = typeof record?.text === "string" ? record.text : null;
  const baseResourcePath =
    descriptor.target.type === "workspace" || descriptor.target.type === "local"
      ? descriptor.target.path
      : descriptor.externalPath;
  const resolveImage = useResourceImageResolver(
    client,
    descriptor.threadId,
    baseResourcePath ?? null,
  );

  if (type === "markdown" && text !== null) {
    return (
      <MarkdownContent
        baseResourcePath={baseResourcePath}
        className="preview-markdown"
        onOpenLink={(href) => onOpenMarkdownLink?.(href, baseResourcePath)}
        onResolveImage={resolveImage}
        text={text}
      />
    );
  }
  if ((type === "text" || type === "code") && text !== null) {
    return (
      <div className="preview-code">
        <MonacoEditor
          language={
            type === "code" && typeof record?.language === "string"
              ? record.language
              : "plaintext"
          }
          readOnly
          theme="vs"
          value={text}
        />
      </div>
    );
  }
  if (
    type === "image" &&
    typeof record?.contentType === "string" &&
    isSafePluginImageType(record.contentType) &&
    typeof record?.contentBase64 === "string"
  ) {
    return (
      <ImagePreviewSurface
        objectUrl={`data:${record.contentType};base64,${record.contentBase64}`}
        title={descriptor.title}
      />
    );
  }

  const serialized =
    typeof payload === "string"
      ? payload
      : (JSON.stringify(payload, null, 2) ?? "No preview output");
  return (
    <div className="preview-code">
      <MonacoEditor
        language={typeof payload === "string" ? "plaintext" : "json"}
        readOnly
        theme="vs"
        value={serialized}
      />
    </div>
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isSafePluginImageType(contentType: string): boolean {
  return /^(?:image\/(?:png|jpeg|gif|webp|avif)|image\/x-icon)$/i.test(
    contentType,
  );
}

function useResourceImageResolver(
  client: ApiClient,
  threadId: string,
  baseResourcePath: string | null,
) {
  return useCallback(
    async (href: string): Promise<Blob | null> => {
      const target = resolveMarkdownLink(href, baseResourcePath);
      if (target.kind !== "workspace" && target.kind !== "local") return null;
      const image = await client.resolvePreview(
        threadId,
        target.kind === "local"
          ? { type: "local", path: target.path }
          : { type: "workspace", path: target.path },
      );
      try {
        if (!image.contentType.toLowerCase().startsWith("image/")) return null;
        return await client.getPreviewContent(image.threadId, image.id);
      } finally {
        await client
          .closePreview(image.threadId, image.id)
          .catch(() => undefined);
      }
    },
    [baseResourcePath, client, threadId],
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
