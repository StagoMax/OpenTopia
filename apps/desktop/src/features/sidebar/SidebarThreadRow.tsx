import { memo, useState } from "react";
import {
  Activity,
  Archive,
  MoreHorizontal,
  Pencil,
  RotateCcw,
} from "lucide-react";
import { SidebarRow, type SidebarRowStatus } from "../../components/ui";
import { useDismissiblePopover } from "../../hooks/useDismissiblePopover";
import {
  isThreadActivityProcessing,
  threadActivityStatusLabel,
  type ThreadActivityStatus,
} from "../../threadActivityStatus";
import type { ThreadActivityStore } from "../../threadActivityStore";
import type { Thread } from "../../types";
import { useThreadActivityStatus } from "../../useThreadActivityStore";

type SidebarThreadRowProps = {
  thread: Thread;
  active: boolean;
  activityStore: ThreadActivityStore;
  onSelect(thread: Thread): void;
  onRename(thread: Thread): void;
  onOpenUsage(thread: Thread): void;
} & (
  | {
      archived: true;
      onArchive?: never;
      onRestore(thread: Thread): void;
    }
  | {
      archived?: false;
      onArchive(thread: Thread): void;
      onRestore?: never;
    }
);

export const SidebarThreadRow = memo(function SidebarThreadRow({
  thread,
  active,
  activityStore,
  archived = false,
  onSelect,
  onRename,
  onOpenUsage,
  onArchive,
  onRestore,
}: SidebarThreadRowProps) {
  const activityStatus = useThreadActivityStatus(activityStore, thread.id);
  const [menuOpen, setMenuOpen] = useState(false);
  const menuRef = useDismissiblePopover(menuOpen, () => setMenuOpen(false));

  return (
    <SidebarRow
      active={active}
      actions={
        <div className="thread-row-menu-wrap" ref={menuRef}>
          <button
            aria-expanded={menuOpen}
            aria-label={`任务菜单 ${thread.title}`}
            className="thread-row-more"
            onClick={() => setMenuOpen((current) => !current)}
            type="button"
          >
            <MoreHorizontal size={13} />
          </button>
          {menuOpen ? (
            <div className="tool-popover thread-row-popover" role="menu">
              <button
                role="menuitem"
                onClick={() => {
                  onRename(thread);
                  setMenuOpen(false);
                }}
              >
                <Pencil size={14} />
                <span>重命名</span>
              </button>
              <button
                role="menuitem"
                onClick={() => {
                  onOpenUsage(thread);
                  setMenuOpen(false);
                }}
              >
                <Activity size={14} />
                <span>使用日志看板</span>
              </button>
              <div className="tool-popover-separator" />
              {archived && onRestore ? (
                <button
                  role="menuitem"
                  onClick={() => {
                    onRestore(thread);
                    setMenuOpen(false);
                  }}
                >
                  <RotateCcw size={14} />
                  <span>恢复到项目</span>
                </button>
              ) : onArchive ? (
                <button
                  role="menuitem"
                  onClick={() => {
                    onArchive(thread);
                    setMenuOpen(false);
                  }}
                >
                  <Archive size={14} />
                  <span>归档任务</span>
                </button>
              ) : null}
            </div>
          ) : null}
        </div>
      }
      actionsOpen={menuOpen}
      className={
        isThreadActivityProcessing(activityStatus) ? "is-processing" : undefined
      }
      onContextMenu={(event) => {
        event.preventDefault();
        setMenuOpen(true);
      }}
      onSelect={() => onSelect(thread)}
      status={sidebarRowStatus(activityStatus)}
      title={thread.title}
    />
  );
});

function sidebarRowStatus(
  status?: ThreadActivityStatus,
): SidebarRowStatus | undefined {
  if (!status) return undefined;
  const label = threadActivityStatusLabel(status);
  if (status === "processing") {
    return { label, loading: true, tone: "info" };
  }
  if (status === "failed") return { label, tone: "danger" };
  if (status === "approval" || status === "user_action") {
    return { label, tone: "warning" };
  }
  return { label, tone: "info" };
}
