export type EditableTextMenuAvailability = {
  canCopy: boolean;
  canCut: boolean;
  canPaste: boolean;
  canSelectAll: boolean;
};

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
