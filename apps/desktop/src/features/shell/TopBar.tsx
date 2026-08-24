import {
  useEffect,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
} from "react";
import {
  ArrowLeft,
  ArrowRight,
  PanelLeftClose,
  PanelLeftOpen,
} from "lucide-react";
import { useDismissiblePopover } from "../../hooks/useDismissiblePopover";
import type { ToolTabKind } from "../../toolTabs";

type TopBarMenu = "file" | "edit" | "view" | "help";
type NativeEditCommand =
  "undo" | "redo" | "cut" | "copy" | "paste" | "delete" | "selectAll";

function isEditableElement(value: EventTarget | null): value is HTMLElement {
  if (value instanceof HTMLTextAreaElement) {
    return !value.disabled && !value.readOnly;
  }
  if (value instanceof HTMLInputElement) {
    return !value.disabled && !value.readOnly;
  }
  return value instanceof HTMLElement && value.isContentEditable;
}

export function TopBar({
  sidebarCollapsed,
  onToggleSidebar,
  onNewWindow,
  onNewChat,
  onOpenWorkspace,
  onCloseWindow,
  onLogout,
  onQuit,
  onToggleTool,
  onOpenSettings,
  onOpenLogs,
  onShowKeyboardShortcuts,
  onShowAbout,
  menuSuppressed,
}: {
  sidebarCollapsed: boolean;
  onToggleSidebar(): void;
  onNewWindow(): void;
  onNewChat(): void;
  onOpenWorkspace(): void;
  onCloseWindow(): void;
  onLogout(): void;
  onQuit(): void;
  onToggleTool(kind: Exclude<ToolTabKind, "preview">): void;
  onOpenSettings(): void;
  onOpenLogs(): void;
  onShowKeyboardShortcuts(): void;
  onShowAbout(): void;
  menuSuppressed: boolean;
}) {
  const [openMenu, setOpenMenu] = useState<TopBarMenu | null>(null);
  const [hasEditableTarget, setHasEditableTarget] = useState(false);
  const editableTargetRef = useRef<HTMLElement | null>(null);
  const menuRef = useDismissiblePopover(Boolean(openMenu), () =>
    setOpenMenu(null),
  );

  useEffect(() => {
    const rememberEditableTarget = (event: FocusEvent) => {
      if (!isEditableElement(event.target)) return;
      editableTargetRef.current = event.target;
      setHasEditableTarget(true);
    };
    document.addEventListener("focusin", rememberEditableTarget);
    return () =>
      document.removeEventListener("focusin", rememberEditableTarget);
  }, []);

  useEffect(() => {
    if (menuSuppressed) setOpenMenu(null);
  }, [menuSuppressed]);

  useEffect(() => {
    setOpenMenu(null);
  }, [sidebarCollapsed]);

  const toggleMenu = (menu: TopBarMenu) => {
    setOpenMenu((current) => (current === menu ? null : menu));
  };
  const closeMenu = () => setOpenMenu(null);
  const runAction = (action: () => void) => {
    action();
    closeMenu();
  };
  const runEditCommand = (command: NativeEditCommand) => {
    const target = editableTargetRef.current;
    if (!target || !target.isConnected || !isEditableElement(target)) {
      setHasEditableTarget(false);
      closeMenu();
      return;
    }
    target.focus({ preventScroll: true });
    if (
      command === "selectAll" &&
      (target instanceof HTMLInputElement ||
        target instanceof HTMLTextAreaElement)
    ) {
      target.select();
    } else {
      document.execCommand(command);
    }
    closeMenu();
  };
  const preserveEditableFocus = (event: ReactPointerEvent<HTMLButtonElement>) =>
    event.preventDefault();

  const menuButton = (menu: TopBarMenu, label: string) => (
    <button
      className={`window-menu-item ${openMenu === menu ? "active" : ""}`}
      type="button"
      aria-haspopup="menu"
      aria-expanded={openMenu === menu}
      onClick={() => toggleMenu(menu)}
    >
      {label}
    </button>
  );

  return (
    <header className="topbar">
      <div className="window-menu" ref={menuRef}>
        <button
          className="window-app-button sidebar-toggle-button"
          type="button"
          aria-label={sidebarCollapsed ? "展开侧栏" : "折叠侧栏"}
          aria-pressed={sidebarCollapsed}
          title={sidebarCollapsed ? "展开侧栏 (Ctrl+B)" : "折叠侧栏 (Ctrl+B)"}
          onClick={() => runAction(onToggleSidebar)}
        >
          {sidebarCollapsed ? (
            <PanelLeftOpen size={15} aria-hidden="true" />
          ) : (
            <PanelLeftClose size={15} aria-hidden="true" />
          )}
          {sidebarCollapsed ? <span className="sidebar-toggle-dot" /> : null}
        </button>
        <button className="window-nav-button" disabled title="后退不可用">
          <ArrowLeft size={14} />
        </button>
        <button className="window-nav-button" disabled title="前进不可用">
          <ArrowRight size={14} />
        </button>

        <div className="window-menu-entry">
          {menuButton("file", "文件")}
          {openMenu === "file" ? (
            <div className="window-menu-popover" role="menu" aria-label="文件">
              <button
                type="button"
                role="menuitem"
                onClick={() => runAction(onNewWindow)}
              >
                <span>新建窗口</span>
              </button>
              <button
                type="button"
                role="menuitem"
                onClick={() => runAction(onNewChat)}
              >
                <span>新聊天</span>
                <kbd>Ctrl+N</kbd>
              </button>
              <div className="window-menu-divider" role="separator" />
              <button
                type="button"
                role="menuitem"
                onClick={() => runAction(onOpenWorkspace)}
              >
                <span>打开文件夹</span>
                <kbd>Ctrl+O</kbd>
              </button>
              <div className="window-menu-divider" role="separator" />
              <button
                type="button"
                role="menuitem"
                onClick={() => runAction(onCloseWindow)}
              >
                <span>关闭</span>
                <kbd>Ctrl+W</kbd>
              </button>
              <div className="window-menu-divider" role="separator" />
              <button
                type="button"
                role="menuitem"
                onClick={() => runAction(onLogout)}
              >
                <span>注销</span>
              </button>
              <button
                type="button"
                role="menuitem"
                onClick={() => runAction(onQuit)}
              >
                <span>退出 ChatGPT</span>
                <kbd>Ctrl+Q</kbd>
              </button>
            </div>
          ) : null}
        </div>

        <div className="window-menu-entry">
          {menuButton("edit", "编辑")}
          {openMenu === "edit" ? (
            <div className="window-menu-popover" role="menu" aria-label="编辑">
              {(
                [
                  ["undo", "撤销", "Ctrl+Z"],
                  ["redo", "重做", "Ctrl+Y"],
                ] as const
              ).map(([command, label, shortcut]) => (
                <button
                  key={command}
                  type="button"
                  role="menuitem"
                  disabled={!hasEditableTarget}
                  onPointerDown={preserveEditableFocus}
                  onClick={() => runEditCommand(command)}
                >
                  <span>{label}</span>
                  <kbd>{shortcut}</kbd>
                </button>
              ))}
              <div className="window-menu-divider" role="separator" />
              {(
                [
                  ["cut", "剪切", "Ctrl+X"],
                  ["copy", "复制", "Ctrl+C"],
                  ["paste", "粘贴", "Ctrl+V"],
                  ["delete", "删除", ""],
                ] as const
              ).map(([command, label, shortcut]) => (
                <button
                  key={command}
                  type="button"
                  role="menuitem"
                  disabled={!hasEditableTarget}
                  onPointerDown={preserveEditableFocus}
                  onClick={() => runEditCommand(command)}
                >
                  <span>{label}</span>
                  <kbd>{shortcut}</kbd>
                </button>
              ))}
              <div className="window-menu-divider" role="separator" />
              <button
                type="button"
                role="menuitem"
                disabled={!hasEditableTarget}
                onPointerDown={preserveEditableFocus}
                onClick={() => runEditCommand("selectAll")}
              >
                <span>全选</span>
                <kbd>Ctrl+A</kbd>
              </button>
              <div className="window-menu-divider" role="separator" />
              <button
                type="button"
                role="menuitem"
                onClick={() => runAction(onOpenSettings)}
              >
                <span>设置</span>
                <kbd>Ctrl+,</kbd>
              </button>
            </div>
          ) : null}
        </div>

        <div className="window-menu-entry">
          {menuButton("view", "视图")}
          {openMenu === "view" ? (
            <div
              className="window-menu-popover window-menu-popover-wide"
              role="menu"
              aria-label="视图"
            >
              <button
                type="button"
                role="menuitem"
                onClick={() => runAction(onToggleSidebar)}
              >
                <span>{sidebarCollapsed ? "展开侧栏" : "折叠侧栏"}</span>
                <kbd>Ctrl+B</kbd>
              </button>
              <button
                type="button"
                role="menuitem"
                onClick={() => runAction(() => onToggleTool("terminal"))}
              >
                <span>打开终端</span>
                <kbd>Ctrl+`</kbd>
              </button>
              <button
                type="button"
                role="menuitem"
                onClick={() => runAction(() => onToggleTool("files"))}
              >
                <span>切换文件树</span>
                <kbd>Ctrl+Shift+E</kbd>
              </button>
              <button
                type="button"
                role="menuitem"
                onClick={() => runAction(() => onToggleTool("diff"))}
              >
                <span>切换审查面板</span>
                <kbd>Ctrl+Alt+B</kbd>
              </button>
              <div className="window-menu-divider" role="separator" />
              <button
                type="button"
                role="menuitem"
                onClick={() => runAction(() => onToggleTool("browser"))}
              >
                <span>新建浏览器</span>
                <kbd>Ctrl+T</kbd>
              </button>
            </div>
          ) : null}
        </div>

        <div className="window-menu-entry">
          {menuButton("help", "帮助")}
          {openMenu === "help" ? (
            <div className="window-menu-popover" role="menu" aria-label="帮助">
              <button
                type="button"
                role="menuitem"
                onClick={() => runAction(onShowKeyboardShortcuts)}
              >
                <span>键盘快捷键</span>
                <kbd>Ctrl+/</kbd>
              </button>
              <div className="window-menu-divider" role="separator" />
              <button
                type="button"
                role="menuitem"
                onClick={() => runAction(onOpenLogs)}
              >
                <span>故障排查（日志）</span>
                <kbd />
              </button>
              <div className="window-menu-divider" role="separator" />
              <button
                type="button"
                role="menuitem"
                onClick={() => runAction(onShowAbout)}
              >
                <span>关于 OpenTopia</span>
                <kbd />
              </button>
            </div>
          ) : null}
        </div>
      </div>
      <div className="topbar-drag-region" aria-hidden="true" />
    </header>
  );
}
