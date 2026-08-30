export type WorkflowCanvasCommand =
  | "deleteSelection"
  | "deselect"
  | "fitView"
  | "openNodePicker"
  | "panTool"
  | "redo"
  | "selectTool"
  | "undo"
  | "zoomIn"
  | "zoomOut";

export const workflowCanvasShortcutLabels = {
  deleteSelection: "Delete",
  fitView: "Shift+1 / F",
  openNodePicker: "N",
  panTool: "H / Space",
  redo: "Ctrl+Shift+Z",
  selectTool: "V",
  undo: "Ctrl+Z",
  zoomIn: "+",
  zoomOut: "−",
} as const;

export const workflowCanvasAriaShortcuts = {
  deleteSelection: "Delete Backspace",
  fitView: "Shift+1 F",
  openNodePicker: "N",
  panTool: "H Space",
  redo: "Control+Shift+Z Meta+Shift+Z Control+Y Meta+Y",
  selectTool: "V",
  undo: "Control+Z Meta+Z",
  zoomIn: "=",
  zoomOut: "-",
} as const;

type ShortcutEvent = Pick<
  KeyboardEvent,
  "altKey" | "code" | "ctrlKey" | "key" | "metaKey" | "shiftKey"
>;

export function workflowCanvasCommand(
  event: ShortcutEvent,
  {
    disabled,
    readOnly,
  }: {
    disabled: boolean;
    readOnly: boolean;
  },
): WorkflowCanvasCommand | null {
  const key = event.key.toLowerCase();
  const primaryModifier = event.ctrlKey || event.metaKey;
  const canEdit = !disabled && !readOnly;

  if (event.key === "Escape") return "deselect";

  if (primaryModifier && !event.altKey) {
    if (!canEdit) return null;
    if (key === "z") return event.shiftKey ? "redo" : "undo";
    if (key === "y") return "redo";
    return null;
  }

  if (event.altKey) return null;

  if (
    event.shiftKey &&
    (event.code === "Digit1" || event.key === "1" || event.key === "!")
  ) {
    return "fitView";
  }
  if (key === "v") return "selectTool";
  if (key === "h") return "panTool";
  if (key === "f") return "fitView";
  if (["+", "=", "add"].includes(key)) return "zoomIn";
  if (["-", "_", "subtract"].includes(key)) return "zoomOut";

  if (!canEdit) return null;
  if (key === "n") return "openNodePicker";
  if (event.key === "Delete" || event.key === "Backspace") {
    return "deleteSelection";
  }

  return null;
}
