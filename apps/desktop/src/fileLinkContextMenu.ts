import { formatPathForDisplay } from "./pathDisplay.ts";

export type ContextMenuPoint = { x: number; y: number };

export type ContextMenuBounds = {
  width: number;
  height: number;
};

export function fitContextMenuPosition(
  point: ContextMenuPoint,
  menu: ContextMenuBounds,
  viewport: ContextMenuBounds,
  margin: number,
): ContextMenuPoint {
  return {
    x: Math.min(
      Math.max(margin, point.x),
      Math.max(margin, viewport.width - menu.width - margin),
    ),
    y: Math.min(
      Math.max(margin, point.y),
      Math.max(margin, viewport.height - menu.height - margin),
    ),
  };
}

/** Removes filesystem-only prefixes before a path reaches the clipboard. */
export function fileLinkClipboardPath(path: string): string {
  return formatPathForDisplay(path);
}
