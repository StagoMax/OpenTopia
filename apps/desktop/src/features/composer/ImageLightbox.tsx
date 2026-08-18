import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import {
  ChevronLeft,
  ChevronRight,
  Download,
  RotateCcw,
  X,
  ZoomIn,
  ZoomOut,
} from "lucide-react";
import { IconButton } from "../../components/ui";
import {
  ImageCopyContextMenu,
  type ImageCopyContextMenuPosition,
} from "../conversation/MessagePartView";

const IMAGE_LIGHTBOX_MIN_ZOOM_PERCENT = 20;
const IMAGE_LIGHTBOX_MAX_ZOOM_PERCENT = 300;
const IMAGE_LIGHTBOX_ZOOM_STEP_PERCENT = 20;
const IMAGE_LIGHTBOX_DEFAULT_ZOOM_PERCENT = 100;

export type ImageLightboxAttachment = {
  previewUrl: string;
  name?: string;
};

export function ImageLightbox({
  attachments,
  activeIndex,
  onChangeIndex,
  onClose,
}: {
  attachments: ImageLightboxAttachment[];
  activeIndex: number;
  onChangeIndex(index: number): void;
  onClose(): void;
}) {
  const [zoomPercent, setZoomPercent] = useState(
    IMAGE_LIGHTBOX_DEFAULT_ZOOM_PERCENT,
  );
  const [copyContextMenu, setCopyContextMenu] =
    useState<ImageCopyContextMenuPosition | null>(null);
  const active = attachments[activeIndex];

  useEffect(() => {
    setZoomPercent(IMAGE_LIGHTBOX_DEFAULT_ZOOM_PERCENT);
    setCopyContextMenu(null);
  }, [activeIndex]);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (copyContextMenu) return;
      if (event.key === "Escape") onClose();
      if (event.key === "ArrowLeft" && activeIndex > 0) {
        onChangeIndex(activeIndex - 1);
      }
      if (event.key === "ArrowRight" && activeIndex < attachments.length - 1) {
        onChangeIndex(activeIndex + 1);
      }
    };
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [
    activeIndex,
    attachments.length,
    copyContextMenu,
    onChangeIndex,
    onClose,
  ]);

  if (!active) return null;
  const activeName = active.name || "图片";

  return createPortal(
    <div
      className="image-lightbox"
      role="dialog"
      aria-modal="true"
      aria-label={`预览 ${activeName}`}
      onPointerDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div className="image-lightbox-dialog">
        <header className="image-lightbox-header">
          <strong>{activeName}</strong>
          <span>
            {activeIndex + 1} / {attachments.length}
          </span>
          <IconButton aria-label="关闭图片预览" title="关闭" onClick={onClose}>
            <X size={18} aria-hidden="true" />
          </IconButton>
        </header>
        <div className="image-lightbox-stage">
          <IconButton
            aria-label="上一张图片"
            title="上一张"
            disabled={activeIndex === 0}
            onClick={() => onChangeIndex(activeIndex - 1)}
          >
            <ChevronLeft size={20} aria-hidden="true" />
          </IconButton>
          <div className="image-lightbox-canvas">
            <img
              src={active.previewUrl}
              alt={activeName}
              draggable={false}
              style={{ transform: `scale(${zoomPercent / 100})` }}
              onContextMenu={(event) => {
                event.preventDefault();
                setCopyContextMenu({ x: event.clientX, y: event.clientY });
              }}
            />
          </div>
          <IconButton
            aria-label="下一张图片"
            title="下一张"
            disabled={activeIndex === attachments.length - 1}
            onClick={() => onChangeIndex(activeIndex + 1)}
          >
            <ChevronRight size={20} aria-hidden="true" />
          </IconButton>
        </div>
        <footer className="image-lightbox-footer">
          <button
            className="image-lightbox-reset"
            type="button"
            onClick={() => setZoomPercent(IMAGE_LIGHTBOX_DEFAULT_ZOOM_PERCENT)}
            disabled={zoomPercent === IMAGE_LIGHTBOX_DEFAULT_ZOOM_PERCENT}
          >
            <RotateCcw size={16} aria-hidden="true" />
            <span>重置</span>
          </button>
          <div className="image-lightbox-zoom-controls">
            <IconButton
              aria-label="缩小图片"
              title="缩小"
              disabled={zoomPercent <= IMAGE_LIGHTBOX_MIN_ZOOM_PERCENT}
              onClick={() =>
                setZoomPercent((current) =>
                  Math.max(
                    IMAGE_LIGHTBOX_MIN_ZOOM_PERCENT,
                    current - IMAGE_LIGHTBOX_ZOOM_STEP_PERCENT,
                  ),
                )
              }
            >
              <ZoomOut size={16} aria-hidden="true" />
            </IconButton>
            <span>{zoomPercent}%</span>
            <IconButton
              aria-label="放大图片"
              title="放大"
              disabled={zoomPercent >= IMAGE_LIGHTBOX_MAX_ZOOM_PERCENT}
              onClick={() =>
                setZoomPercent((current) =>
                  Math.min(
                    IMAGE_LIGHTBOX_MAX_ZOOM_PERCENT,
                    current + IMAGE_LIGHTBOX_ZOOM_STEP_PERCENT,
                  ),
                )
              }
            >
              <ZoomIn size={16} aria-hidden="true" />
            </IconButton>
          </div>
          <a
            className="image-lightbox-download"
            href={active.previewUrl}
            download={activeName}
          >
            <Download size={15} aria-hidden="true" />
            <span>下载</span>
          </a>
        </footer>
      </div>
      {copyContextMenu ? (
        <ImageCopyContextMenu
          name={activeName}
          position={copyContextMenu}
          previewUrl={active.previewUrl}
          onClose={() => setCopyContextMenu(null)}
        />
      ) : null}
    </div>,
    document.body,
  );
}
