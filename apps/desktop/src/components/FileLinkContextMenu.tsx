import {
  AppWindow,
  CircleAlert,
  Code2,
  Copy,
  FileOutput,
  FileText,
  FolderSearch,
  Save,
} from "lucide-react";
import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";
import { createPortal } from "react-dom";
import {
  fileLinkClipboardPath,
  fitContextMenuPosition,
  type ContextMenuPoint,
} from "../fileLinkContextMenu";
import { performFileLinkAction } from "../platform";
import type { FileLinkAction } from "../types";
import "./FileLinkContextMenu.css";

type MenuAction = FileLinkAction | "open" | "copy-path" | "copy-content";

export type FileLinkContextMenuProps = {
  path: string;
  line?: number | null;
  point: ContextMenuPoint;
  onOpen?(): void;
  readText?(): Promise<string>;
  onClose(options?: { restoreFocus?: boolean }): void;
};

const menuItems: Array<
  | { type: "separator" }
  | {
      type: "action";
      action: MenuAction;
      label: string;
      icon: typeof FileText;
    }
> = [
  { type: "action", action: "open", label: "打开文件", icon: FileText },
  {
    type: "action",
    action: "open-vscode",
    label: "在 VS Code 中打开",
    icon: Code2,
  },
  {
    type: "action",
    action: "open-with",
    label: "使用其他应用打开…",
    icon: AppWindow,
  },
  { type: "separator" },
  { type: "action", action: "save-as", label: "另存为…", icon: Save },
  { type: "action", action: "copy-path", label: "复制路径", icon: Copy },
  {
    type: "action",
    action: "copy-content",
    label: "复制文件内容",
    icon: FileOutput,
  },
  {
    type: "action",
    action: "reveal",
    label: "在资源管理器中显示",
    icon: FolderSearch,
  },
];

export function FileLinkContextMenu({
  path,
  line,
  point,
  onOpen,
  readText,
  onClose,
}: FileLinkContextMenuProps) {
  const menuRef = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState<ContextMenuPoint | null>(null);
  const [pendingAction, setPendingAction] = useState<MenuAction | null>(null);
  const [error, setError] = useState<string | null>(null);

  useLayoutEffect(() => {
    const menu = menuRef.current;
    if (!menu) return;
    const styles = getComputedStyle(document.documentElement);
    const margin = Number.parseFloat(styles.getPropertyValue("--space-4")) || 0;
    const bounds = menu.getBoundingClientRect();
    setPosition(
      fitContextMenuPosition(
        point,
        { width: bounds.width, height: bounds.height },
        { width: window.innerWidth, height: window.innerHeight },
        margin,
      ),
    );
  }, [error, point]);

  useLayoutEffect(() => {
    if (!position) return;
    menuRef.current
      ?.querySelector<HTMLButtonElement>('[role="menuitem"]')
      ?.focus();
  }, [position]);

  useEffect(() => {
    function handlePointerDown(event: PointerEvent) {
      if (menuRef.current?.contains(event.target as Node)) return;
      onClose();
    }

    function handleKeyDown(event: globalThis.KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        onClose({ restoreFocus: true });
      } else if (event.key === "Tab") {
        onClose();
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
      onClose();
    }
  }, [onClose]);

  async function runAction(action: MenuAction) {
    if (pendingAction) return;
    setPendingAction(action);
    setError(null);
    try {
      if (action === "open" && onOpen) {
        onOpen();
      } else if (action === "copy-path") {
        await writeClipboardText(fileLinkClipboardPath(path));
      } else if (action === "copy-content") {
        if (!readText) throw new Error("未能读取文件内容。");
        await writeClipboardText(await readText());
      } else {
        await performFileLinkAction({
          action: action === "open" ? "open-default" : action,
          path,
          line,
        });
      }
      onClose();
    } catch (cause) {
      setError(fileActionError(cause));
    } finally {
      setPendingAction(null);
    }
  }

  function handleMenuKeyDown(event: KeyboardEvent<HTMLDivElement>) {
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

  return createPortal(
    <div
      aria-label="文件操作"
      className="file-link-context-menu"
      onContextMenu={(event) => event.preventDefault()}
      onKeyDown={handleMenuKeyDown}
      ref={menuRef}
      role="menu"
      style={{
        left: position?.x ?? point.x,
        top: position?.y ?? point.y,
        visibility: position ? "visible" : "hidden",
      }}
    >
      {menuItems.map((item, index) => {
        if (item.type === "separator") {
          return (
            <div
              aria-hidden="true"
              className="file-link-context-menu__separator"
              key={`separator-${index}`}
              role="separator"
            />
          );
        }
        const Icon = item.icon;
        return (
          <button
            className="file-link-context-menu__item"
            disabled={pendingAction !== null}
            key={item.action}
            onClick={() => void runAction(item.action)}
            role="menuitem"
            type="button"
          >
            <Icon aria-hidden="true" size={16} />
            <span>{item.label}</span>
          </button>
        );
      })}
      {error ? (
        <p className="file-link-context-menu__error" role="alert">
          <CircleAlert aria-hidden="true" size={14} />
          <span>{error}</span>
        </p>
      ) : null}
    </div>,
    document.body,
  );
}

async function writeClipboardText(text: string): Promise<void> {
  if (!navigator.clipboard?.writeText) {
    throw new Error("当前环境不支持文本复制。");
  }
  await navigator.clipboard.writeText(text);
}

function fileActionError(cause: unknown): string {
  const message = cause instanceof Error ? cause.message : String(cause);
  if (/too large/i.test(message)) return "文件过大，无法完整复制内容。";
  if (/binary/i.test(message)) return "二进制文件无法复制为文本。";
  return message || "文件操作失败，请重试。";
}
