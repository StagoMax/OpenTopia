import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import { createPortal } from "react-dom";
import {
  editableTextMenuAvailability,
  type EditableTextMenuAvailability,
} from "../../editableTextContextMenu";
import {
  fitContextMenuPosition,
  type ContextMenuPoint,
} from "../../fileLinkContextMenu";

type TextControl = HTMLInputElement | HTMLTextAreaElement;
type EditableTarget = TextControl | HTMLElement;
type EditAction = "cut" | "copy" | "paste" | "selectAll";

type SelectionSnapshot =
  | {
      kind: "control";
      start: number;
      end: number;
      direction: "forward" | "backward" | "none";
    }
  | { kind: "content"; range: Range };

type MenuState = {
  availability: EditableTextMenuAvailability;
  point: ContextMenuPoint;
  selectedText: string;
  selection: SelectionSnapshot;
  target: EditableTarget;
};

const textInputTypes = new Set([
  "email",
  "password",
  "search",
  "tel",
  "text",
  "url",
]);

const menuItems: Array<{ action: EditAction; label: string }> = [
  { action: "cut", label: "剪切" },
  { action: "copy", label: "复制" },
  { action: "paste", label: "粘贴" },
  { action: "selectAll", label: "全选" },
];

export function EditableTextContextMenu() {
  const menuRef = useRef<HTMLDivElement>(null);
  const activeMenuRef = useRef<MenuState | null>(null);
  const [menu, setMenu] = useState<MenuState | null>(null);
  const [position, setPosition] = useState<ContextMenuPoint | null>(null);

  const close = useCallback((restoreFocus = false) => {
    const current = activeMenuRef.current;
    activeMenuRef.current = null;
    if (restoreFocus && current) restoreSelection(current);
    setMenu(null);
    setPosition(null);
  }, []);

  useEffect(() => {
    function handleContextMenu(event: MouseEvent) {
      const target = editableTargetFrom(event.target);
      if (!target) return;
      const selection = captureSelection(target);
      if (!selection) return;

      event.preventDefault();
      const bounds = target.getBoundingClientRect();
      const point =
        event.clientX === 0 && event.clientY === 0
          ? { x: bounds.left, y: bounds.bottom }
          : { x: event.clientX, y: event.clientY };
      const selectedText = selectedTextFor(target, selection);
      const nextMenu = {
        availability: editableTextMenuAvailability({
          readOnly: isReadOnly(target),
          selectionLength: selectedText.length,
          textLength: editableTextLength(target),
        }),
        point,
        selectedText,
        selection,
        target,
      };
      activeMenuRef.current = nextMenu;
      setPosition(null);
      setMenu(nextMenu);
    }

    document.addEventListener("contextmenu", handleContextMenu, true);
    return () =>
      document.removeEventListener("contextmenu", handleContextMenu, true);
  }, []);

  useLayoutEffect(() => {
    if (!menu) return;
    const menuElement = menuRef.current;
    if (!menuElement) return;
    const styles = getComputedStyle(document.documentElement);
    const margin = Number.parseFloat(styles.getPropertyValue("--space-4")) || 0;
    const bounds = menuElement.getBoundingClientRect();
    setPosition(
      fitContextMenuPosition(
        menu.point,
        { width: bounds.width, height: bounds.height },
        { width: window.innerWidth, height: window.innerHeight },
        margin,
      ),
    );
  }, [menu]);

  useLayoutEffect(() => {
    if (!position) return;
    menuRef.current
      ?.querySelector<HTMLButtonElement>('[role="menuitem"]:not(:disabled)')
      ?.focus();
  }, [position]);

  useEffect(() => {
    if (!menu) return undefined;

    function handlePointerDown(event: PointerEvent) {
      if (menuRef.current?.contains(event.target as Node)) return;
      close();
    }

    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        close(true);
      } else if (event.key === "Tab") {
        close();
      }
    }

    document.addEventListener("pointerdown", handlePointerDown);
    document.addEventListener("keydown", handleKeyDown);
    window.addEventListener("blur", handleWindowChange);
    window.addEventListener("resize", handleWindowChange);
    window.addEventListener("scroll", handleWindowChange, true);
    return () => {
      document.removeEventListener("pointerdown", handlePointerDown);
      document.removeEventListener("keydown", handleKeyDown);
      window.removeEventListener("blur", handleWindowChange);
      window.removeEventListener("resize", handleWindowChange);
      window.removeEventListener("scroll", handleWindowChange, true);
    };

    function handleWindowChange() {
      close();
    }
  }, [close, menu]);

  async function runAction(action: EditAction) {
    if (!menu || !menu.target.isConnected) return close();
    const current = menu;
    close();
    restoreSelection(current);

    if (action === "selectAll") {
      selectAll(current.target);
      return;
    }
    if (action === "copy") {
      await copyText(current.selectedText);
      return;
    }
    if (action === "cut") {
      if (!(await copyText(current.selectedText))) return;
      if (!current.target.isConnected) return;
      restoreSelection(current);
      if (!document.execCommand("delete")) {
        deleteSelection(current);
      }
      return;
    }

    if (!navigator.clipboard?.readText) {
      document.execCommand("paste");
      return;
    }
    const text = await readClipboardText();
    if (text === null || !current.target.isConnected) return;
    restoreSelection(current);
    if (!document.execCommand("insertText", false, text)) {
      replaceSelection(current, text, "insertFromPaste");
    }
  }

  function handleMenuKeyDown(event: ReactKeyboardEvent<HTMLDivElement>) {
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
      return;
    }
    const items = Array.from(
      event.currentTarget.querySelectorAll<HTMLButtonElement>(
        '[role="menuitem"]:not(:disabled)',
      ),
    );
    if (items.length === 0) return;
    event.preventDefault();
    const currentIndex = items.indexOf(
      document.activeElement as HTMLButtonElement,
    );
    const nextIndex =
      event.key === "Home"
        ? 0
        : event.key === "End"
          ? items.length - 1
          : event.key === "ArrowDown"
            ? (currentIndex + 1) % items.length
            : (currentIndex - 1 + items.length) % items.length;
    items[nextIndex]?.focus();
  }

  if (!menu) return null;
  return createPortal(
    <div
      aria-label="文本编辑"
      className="ot-editable-context-menu"
      onContextMenu={(event) => event.preventDefault()}
      onKeyDown={handleMenuKeyDown}
      ref={menuRef}
      role="menu"
      style={{
        left: position?.x ?? menu.point.x,
        top: position?.y ?? menu.point.y,
        visibility: position ? "visible" : "hidden",
      }}
    >
      {menuItems.map(({ action, label }) => (
        <button
          className="ot-editable-context-menu__item"
          disabled={!menu.availability[actionAvailabilityKey(action)]}
          key={action}
          onClick={() => void runAction(action)}
          role="menuitem"
          type="button"
        >
          {label}
        </button>
      ))}
    </div>,
    document.body,
  );
}

function editableTargetFrom(value: EventTarget | null): EditableTarget | null {
  const element =
    value instanceof Element
      ? value
      : value instanceof Node
        ? value.parentElement
        : null;
  if (!element) return null;

  const control = element.closest("input, textarea");
  if (control instanceof HTMLTextAreaElement) {
    return control.disabled ? null : control;
  }
  if (control instanceof HTMLInputElement) {
    return !control.disabled && textInputTypes.has(control.type)
      ? control
      : null;
  }

  const editable = element.closest<HTMLElement>("[contenteditable]");
  if (
    !editable ||
    editable.contentEditable === "false" ||
    !editable.isContentEditable
  ) {
    return null;
  }
  return editable;
}

function captureSelection(target: EditableTarget): SelectionSnapshot | null {
  if (isTextControl(target)) {
    if (target.selectionStart === null || target.selectionEnd === null) {
      return null;
    }
    return {
      kind: "control",
      start: target.selectionStart,
      end: target.selectionEnd,
      direction: target.selectionDirection ?? "none",
    };
  }

  const selection = window.getSelection();
  if (
    selection &&
    selection.rangeCount > 0 &&
    target.contains(selection.getRangeAt(0).commonAncestorContainer)
  ) {
    return { kind: "content", range: selection.getRangeAt(0).cloneRange() };
  }
  const range = document.createRange();
  range.selectNodeContents(target);
  range.collapse(false);
  return { kind: "content", range };
}

function restoreSelection(menu: MenuState) {
  const { selection, target } = menu;
  if (!target.isConnected) return;
  target.focus({ preventScroll: true });
  if (selection.kind === "control" && isTextControl(target)) {
    target.setSelectionRange(
      selection.start,
      selection.end,
      selection.direction,
    );
    return;
  }
  if (selection.kind === "content" && !isTextControl(target)) {
    const current = window.getSelection();
    current?.removeAllRanges();
    current?.addRange(selection.range);
  }
}

function selectedTextFor(
  target: EditableTarget,
  selection: SelectionSnapshot,
): string {
  if (selection.kind === "control" && isTextControl(target)) {
    return target.value.slice(selection.start, selection.end);
  }
  return selection.kind === "content" ? selection.range.toString() : "";
}

function editableTextLength(target: EditableTarget): number {
  return isTextControl(target)
    ? target.value.length
    : (target.textContent?.length ?? 0);
}

function isReadOnly(target: EditableTarget): boolean {
  return isTextControl(target)
    ? target.readOnly
    : target.getAttribute("aria-readonly") === "true";
}

function isTextControl(target: EditableTarget): target is TextControl {
  return (
    target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement
  );
}

function actionAvailabilityKey(
  action: EditAction,
): keyof EditableTextMenuAvailability {
  if (action === "cut") return "canCut";
  if (action === "copy") return "canCopy";
  if (action === "paste") return "canPaste";
  return "canSelectAll";
}

function selectAll(target: EditableTarget) {
  if (isTextControl(target)) {
    target.select();
    return;
  }
  const range = document.createRange();
  range.selectNodeContents(target);
  const selection = window.getSelection();
  selection?.removeAllRanges();
  selection?.addRange(range);
}

async function copyText(text: string): Promise<boolean> {
  if (navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      // Fall through to Chromium's focused-selection command.
    }
  }
  return document.execCommand("copy");
}

async function readClipboardText(): Promise<string | null> {
  if (navigator.clipboard?.readText) {
    try {
      return await navigator.clipboard.readText();
    } catch {
      return null;
    }
  }
  return null;
}

function deleteSelection(menu: MenuState) {
  replaceSelection(menu, "", "deleteByCut");
}

function replaceSelection(
  menu: MenuState,
  text: string,
  inputType: "deleteByCut" | "insertFromPaste",
) {
  const { selection, target } = menu;
  const eventOptions = {
    bubbles: true,
    data: text || null,
    inputType,
  };
  if (
    !target.dispatchEvent(
      new InputEvent("beforeinput", {
        ...eventOptions,
        cancelable: true,
      }),
    )
  ) {
    return;
  }
  if (selection.kind === "control" && isTextControl(target)) {
    target.setRangeText(text, selection.start, selection.end, "end");
  } else if (selection.kind === "content" && !isTextControl(target)) {
    selection.range.deleteContents();
    if (text) {
      const textNode = document.createTextNode(text);
      selection.range.insertNode(textNode);
      selection.range.setStartAfter(textNode);
      selection.range.collapse(true);
    }
  }
  target.dispatchEvent(new InputEvent("input", eventOptions));
}
