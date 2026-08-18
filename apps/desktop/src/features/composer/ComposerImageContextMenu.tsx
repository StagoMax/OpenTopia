import { createPortal } from "react-dom";
import { Quote, Trash2 } from "lucide-react";
import type { Ref } from "react";

export type ComposerImageContextMenuState = {
  imageId: string;
  x: number;
  y: number;
};

export function ComposerImageContextMenu({
  state,
  menuRef,
  onQuote,
  onRemove,
}: {
  state: ComposerImageContextMenuState;
  menuRef: Ref<HTMLDivElement>;
  onQuote(): void;
  onRemove(): void;
}) {
  return createPortal(
    <div
      ref={menuRef}
      className="tool-popover composer-image-context-menu"
      role="menu"
      style={{ left: state.x, top: state.y }}
      onPointerDown={(event) => event.stopPropagation()}
    >
      <button role="menuitem" onClick={onQuote}>
        <Quote size={14} aria-hidden="true" />
        <span>引用</span>
      </button>
      <button role="menuitem" onClick={onRemove}>
        <Trash2 size={14} aria-hidden="true" />
        <span>删除此引用</span>
      </button>
    </div>,
    document.body,
  );
}
