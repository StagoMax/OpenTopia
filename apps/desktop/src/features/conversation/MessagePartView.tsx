import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Check, Copy, ExternalLink, Loader2, Plug } from "lucide-react";
import { FileTypeIcon } from "../../components/FileTypeIcon";
import { MarkdownContent } from "../../components/MarkdownContent";
import {
  artifactReferencesFromText,
  type ArtifactReference,
} from "../../artifactReferences";
import { formatBytes } from "../../formatBytes";
import { openPath, writeClipboardImage } from "../../platform";
import type {
  ArtifactDescriptor,
  ContextSourceRef,
  Message,
  MessagePart,
} from "../../types";

export function MessagePartView({
  attachmentSources,
  messageId,
  part,
  referencedImage,
  imagePreviewUrl,
  onPreviewImage,
  role,
  threadId,
  artifacts,
  onOpenArtifact,
  onOpenAttachmentPreview,
  onOpenMarkdownLink,
}: {
  attachmentSources: ContextSourceRef[];
  messageId: string;
  part: MessagePart;
  referencedImage?: Extract<MessagePart, { type: "image" }>;
  imagePreviewUrl?: string;
  onPreviewImage?(): void;
  role: Message["role"];
  threadId: string;
  artifacts: ArtifactDescriptor[];
  onOpenArtifact(artifactId: string): void;
  onOpenAttachmentPreview(source: ContextSourceRef): void;
  onOpenMarkdownLink(href: string): void;
}) {
  if (part.type === "image") {
    return (
      <InlineImageMessagePart
        part={part}
        previewUrl={imagePreviewUrl}
        onPreview={onPreviewImage}
      />
    );
  }
  if (part.type === "image_ref") {
    return referencedImage ? (
      <InlineImageMessagePart
        part={referencedImage}
        previewUrl={imagePreviewUrl}
        onPreview={onPreviewImage}
        compact
      />
    ) : (
      <span className="message-image-reference-missing">[图片不可用]</span>
    );
  }
  if (part.type === "text") {
    const refs = artifactReferencesFromText(part.text);
    return (
      <>
        {role === "assistant" ? (
          <MarkdownContent
            attachmentSources={attachmentSources}
            className="message-markdown"
            onOpenAttachment={onOpenAttachmentPreview}
            onOpenLink={onOpenMarkdownLink}
            renderTrace={{
              channel: "assistant",
              threadId,
              messageId,
            }}
            text={part.text}
          />
        ) : (
          <span className="message-text">{part.text}</span>
        )}
        <MessageArtifactLinks
          refs={refs}
          artifacts={artifacts}
          onOpenArtifact={onOpenArtifact}
        />
      </>
    );
  }
  if (part.type === "error") {
    return <p className="message-error">{part.message}</p>;
  }
  if (part.type === "file_ref") return <code>{part.path}</code>;
  if (part.type === "source_ref") {
    return (
      <button
        className="message-source-reference"
        type="button"
        title={`在右侧预览 ${part.source.name}`}
        onClick={() => onOpenAttachmentPreview(part.source)}
      >
        <FileTypeIcon
          name={part.source.name}
          contentType={part.source.contentType}
        />
        <span>{part.source.name}</span>
        <small>{formatBytes(part.source.bytes)}</small>
      </button>
    );
  }
  if (part.type === "skill_ref") {
    return (
      <button
        className="message-source-reference is-skill"
        type="button"
        title={part.skill.description || part.skill.path}
        onClick={() => void openPath(part.skill.path)}
      >
        <Plug size={12} />
        <span>{part.skill.name}</span>
        <small>Skill</small>
      </button>
    );
  }
  return null;
}

function InlineImageMessagePart({
  part,
  previewUrl,
  onPreview,
  compact = false,
}: {
  part: Extract<MessagePart, { type: "image" }>;
  previewUrl?: string;
  onPreview?(): void;
  compact?: boolean;
}) {
  const [copyContextMenu, setCopyContextMenu] = useState<{
    x: number;
    y: number;
  } | null>(null);

  if (!previewUrl) return null;
  const name = part.name || "图片";

  return (
    <>
      <button
        className={`message-inline-image ${compact ? "is-reference" : ""}`}
        data-text-context-menu="custom"
        type="button"
        aria-controls="workspace-right-panel"
        aria-label={`在右侧预览 ${name}`}
        title={`在右侧预览 ${name}`}
        onClick={onPreview}
        onContextMenu={(event) => {
          event.preventDefault();
          setCopyContextMenu({ x: event.clientX, y: event.clientY });
        }}
      >
        <img src={previewUrl} alt={name} decoding="async" loading="lazy" />
      </button>
      {copyContextMenu ? (
        <ImageCopyContextMenu
          name={name}
          position={copyContextMenu}
          previewUrl={previewUrl}
          onClose={() => setCopyContextMenu(null)}
        />
      ) : null}
    </>
  );
}

export type ImageCopyContextMenuPosition = {
  x: number;
  y: number;
};

type ImageCopyStatus = "idle" | "copying" | "copied" | "error";

export function ImageCopyContextMenu({
  name,
  position,
  previewUrl,
  onClose,
}: {
  name: string;
  position: ImageCopyContextMenuPosition;
  previewUrl: string;
  onClose(): void;
}) {
  const menuRef = useRef<HTMLDivElement>(null);
  const [resolvedPosition, setResolvedPosition] = useState(position);
  const [status, setStatus] = useState<ImageCopyStatus>("idle");

  useEffect(() => {
    setResolvedPosition(position);
    setStatus("idle");
  }, [position, previewUrl]);

  useEffect(() => {
    const closeOnPointerDown = () => onClose();
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("pointerdown", closeOnPointerDown);
    window.addEventListener("blur", onClose);
    window.addEventListener("keydown", closeOnEscape);
    return () => {
      window.removeEventListener("pointerdown", closeOnPointerDown);
      window.removeEventListener("blur", onClose);
      window.removeEventListener("keydown", closeOnEscape);
    };
  }, [onClose]);

  useEffect(() => {
    if (status !== "copied") return;
    const timer = window.setTimeout(onClose, 800);
    return () => window.clearTimeout(timer);
  }, [onClose, status]);

  useLayoutEffect(() => {
    const menu = menuRef.current;
    if (!menu) return;
    const bounds = menu.getBoundingClientRect();
    const nextPosition = {
      x: Math.max(
        0,
        resolvedPosition.x - Math.max(0, bounds.right - window.innerWidth),
      ),
      y: Math.max(
        0,
        resolvedPosition.y - Math.max(0, bounds.bottom - window.innerHeight),
      ),
    };
    if (
      nextPosition.x !== resolvedPosition.x ||
      nextPosition.y !== resolvedPosition.y
    ) {
      setResolvedPosition(nextPosition);
      return;
    }
    menu.querySelector<HTMLButtonElement>("button")?.focus();
  }, [resolvedPosition]);

  const label =
    status === "copying"
      ? "正在复制…"
      : status === "copied"
        ? "已复制图片"
        : status === "error"
          ? "复制失败，请重试"
          : "复制图片";

  return createPortal(
    <div
      ref={menuRef}
      className="tool-popover image-copy-context-menu"
      role="menu"
      aria-label={`${name} 操作`}
      data-status={status}
      style={{ left: resolvedPosition.x, top: resolvedPosition.y }}
      onContextMenu={(event) => event.preventDefault()}
      onPointerDown={(event) => event.stopPropagation()}
    >
      <button
        role="menuitem"
        type="button"
        disabled={status === "copying" || status === "copied"}
        onClick={() => {
          setStatus("copying");
          void copyImageToClipboard(previewUrl)
            .then(() => setStatus("copied"))
            .catch(() => setStatus("error"));
        }}
      >
        {status === "copying" ? (
          <Loader2 className="spin" size={14} aria-hidden="true" />
        ) : status === "copied" ? (
          <Check size={14} aria-hidden="true" />
        ) : (
          <Copy size={14} aria-hidden="true" />
        )}
        <span aria-live="polite">{label}</span>
      </button>
    </div>,
    document.body,
  );
}

async function copyImageToClipboard(previewUrl: string): Promise<void> {
  const pngBlob = await imagePreviewToPngBlob(previewUrl);
  await writeClipboardImage(pngBlob);
}

async function imagePreviewToPngBlob(previewUrl: string): Promise<Blob> {
  const image = new Image();
  image.src = previewUrl;
  await image.decode();
  const canvas = document.createElement("canvas");
  canvas.width = image.naturalWidth;
  canvas.height = image.naturalHeight;
  const context = canvas.getContext("2d");
  if (!context) throw new Error("无法创建图片复制画布");
  context.drawImage(image, 0, 0);
  return new Promise<Blob>((resolve, reject) => {
    canvas.toBlob((blob) => {
      if (blob) resolve(blob);
      else reject(new Error("无法转换待复制图片"));
    }, "image/png");
  });
}

function MessageArtifactLinks({
  refs,
  artifacts,
  onOpenArtifact,
}: {
  refs: ArtifactReference[];
  artifacts: ArtifactDescriptor[];
  onOpenArtifact(artifactId: string): void;
}) {
  if (!refs.length) return null;
  return (
    <div className="message-artifact-links">
      {refs.map((ref) => {
        const descriptor = artifacts.find((artifact) => artifact.id === ref.id);
        return (
          <button
            className="artifact-reference-button"
            key={ref.id}
            type="button"
            title={ref.id}
            onClick={() => onOpenArtifact(ref.id)}
          >
            <ExternalLink size={12} />
            <span>{descriptor?.kind ?? ref.kind ?? "artifact"}</span>
            <small>
              {descriptor?.bytes
                ? formatBytes(descriptor.bytes)
                : ref.bytes
                  ? formatBytes(ref.bytes)
                  : "load"}
            </small>
          </button>
        );
      })}
    </div>
  );
}
