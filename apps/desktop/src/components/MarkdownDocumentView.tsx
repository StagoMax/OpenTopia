import { useEffect, useRef, useState } from "react";
import { AlertCircle, Loader2, RotateCcw, Save } from "lucide-react";

import { ApiResponseError, type ApiClient } from "../api/client";
import type {
  PreviewSessionStore,
  PreviewViewMode,
} from "../previewSessionStore";
import type { PreviewDescriptor } from "../types";
import { MarkdownContent } from "./MarkdownContent";
import { MonacoEditor } from "./MonacoEditor";
import { Badge, Button, SegmentedControl } from "./ui";
import "./MarkdownDocumentView.css";

const markdownViewModes: ReadonlyArray<{
  value: PreviewViewMode;
  label: string;
}> = [
  { value: "preview", label: "预览" },
  { value: "source", label: "源码" },
  { value: "split", label: "分屏" },
];

export function MarkdownDocumentView({
  baseResourcePath,
  client,
  descriptor,
  loadedText,
  onDirtyChange,
  onOpenMarkdownLink,
  previewSessionStore,
  resolveImage,
  sessionId,
}: {
  baseResourcePath: string | null;
  client: ApiClient;
  descriptor: PreviewDescriptor;
  loadedText: string;
  onDirtyChange?(dirty: boolean): void;
  onOpenMarkdownLink?(href: string, baseResourcePath?: string | null): void;
  previewSessionStore?: PreviewSessionStore;
  resolveImage?(href: string): Promise<Blob | null>;
  sessionId?: string;
}) {
  const storedSession = sessionId
    ? previewSessionStore?.get(sessionId)
    : undefined;
  const restoredDraft = Boolean(storedSession?.dirty);
  const [mode, setMode] = useState<PreviewViewMode>(
    storedSession?.mode ?? "preview",
  );
  const [draft, setDraft] = useState(
    restoredDraft ? storedSession!.draft : loadedText,
  );
  const [baseline, setBaseline] = useState(
    restoredDraft ? storedSession!.baseline : loadedText,
  );
  const [revision, setRevision] = useState(
    restoredDraft ? storedSession!.revision : descriptor.revision,
  );
  const [externalChanged, setExternalChanged] = useState(
    Boolean(
      storedSession?.externalChanged ||
      (restoredDraft && storedSession?.revision !== descriptor.revision),
    ),
  );
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const currentRef = useRef({ draft, baseline, revision });
  const dirty = draft !== baseline;

  useEffect(() => {
    currentRef.current = { draft, baseline, revision };
  }, [baseline, draft, revision]);

  useEffect(() => {
    onDirtyChange?.(dirty);
  }, [dirty, onDirtyChange]);

  useEffect(() => {
    if (!sessionId || !previewSessionStore) return;
    previewSessionStore.set(sessionId, {
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
    previewSessionStore,
    revision,
    sessionId,
  ]);

  async function readLatest(): Promise<{
    text: string;
    descriptor: PreviewDescriptor;
  }> {
    const latestDescriptor = await client.getResourceMetadata(descriptor);
    const blob = await client.getPreviewContent(
      descriptor.threadId,
      descriptor.id,
    );
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
        setMessage(
          "文件已被其他程序修改。请重新载入后再保存。当前草稿不会丢失。",
        );
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
        // Transient polling failures should not interrupt editing. Explicit
        // save and reload actions surface actionable errors.
      } finally {
        checking = false;
      }
    };
    const timer = window.setInterval(() => void check(), 2_500);
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, [
    client,
    descriptor.capabilities.watch,
    descriptor.id,
    descriptor.revision,
    descriptor.threadId,
  ]);

  useEffect(() => {
    const handleSaveShortcut = (event: KeyboardEvent) => {
      if (
        !(event.ctrlKey || event.metaKey) ||
        event.key.toLowerCase() !== "s"
      ) {
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

function errorMessage(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}
