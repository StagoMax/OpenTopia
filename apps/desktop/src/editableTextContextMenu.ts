export type EditableTextMenuAvailability = {
  canCopy: boolean;
  canCut: boolean;
  canPaste: boolean;
  canSelectAll: boolean;
};

export type TextContextMenuAction = "cut" | "copy" | "paste" | "selectAll";
export type TextContextMenuMode = "editable" | "selection";

const editableActions: readonly TextContextMenuAction[] = [
  "cut",
  "copy",
  "paste",
  "selectAll",
];
const selectionActions: readonly TextContextMenuAction[] = [
  "copy",
  "selectAll",
];

export function textContextMenuActions(
  mode: TextContextMenuMode,
): readonly TextContextMenuAction[] {
  return mode === "editable" ? editableActions : selectionActions;
}

export function editableTextMenuAvailability(options: {
  readOnly: boolean;
  selectionLength: number;
  textLength: number;
}): EditableTextMenuAvailability {
  const hasSelection = options.selectionLength > 0;
  return {
    canCopy: hasSelection,
    canCut: !options.readOnly && hasSelection,
    canPaste: !options.readOnly,
    canSelectAll: options.textLength > 0,
  };
}
