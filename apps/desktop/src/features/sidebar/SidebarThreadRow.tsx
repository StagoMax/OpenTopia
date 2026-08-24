import { memo, useEffect, useRef, useState, type CSSProperties } from "react";
import {
  Activity,
  Archive,
  Circle,
  CircleAlert,
  Loader2,
  MoreHorizontal,
  Pencil,
  RotateCcw,
} from "lucide-react";
import { useDismissiblePopover } from "../../hooks/useDismissiblePopover";
import {
  isThreadActivityProcessing,
  threadActivityStatusLabel,
  type ThreadActivityStatus,
} from "../../threadActivityStatus";
import type { ThreadActivityStore } from "../../threadActivityStore";
import { threadTitleScrollDurationMs } from "../../threadTitleScroll";
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

function ThreadStatusIndicator({ status }: { status?: ThreadActivityStatus }) {
  if (!status) return null;
  const label = threadActivityStatusLabel(status);

  return (
    <span
      className={`thread-row-status is-${status}`}
      role="img"
      aria-label={label}
      title={label}
    >
      {status === "processing" ? (
        <Loader2
          size={14}
          className="thread-status-spinner"
          aria-hidden="true"
        />
      ) : status === "failed" ? (
        <CircleAlert size={14} aria-hidden="true" />
      ) : (
        <Circle size={9} fill="currentColor" aria-hidden="true" />
      )}
    </span>
  );
}

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
  const [titleOverflow, setTitleOverflow] = useState({
    distance: 0,
    durationMs: 0,
  });
  const titleViewportRef = useRef<HTMLSpanElement>(null);
  const titleTextRef = useRef<HTMLSpanElement>(null);
  const menuRef = useDismissiblePopover(menuOpen, () => setMenuOpen(false));

  useEffect(() => {
    const viewport = titleViewportRef.current;
    const text = titleTextRef.current;
    if (!viewport || !text) return;

    const measure = () => {
      const distance = Math.max(
        0,
        Math.ceil(text.scrollWidth - viewport.clientWidth),
      );
      const durationMs = threadTitleScrollDurationMs(distance);
      setTitleOverflow((current) =>
        current.distance === distance && current.durationMs === durationMs
          ? current
          : { distance, durationMs },
      );
    };
    const frame = window.requestAnimationFrame(measure);
    const observer = new ResizeObserver(measure);
    observer.observe(viewport);
    observer.observe(text);
    return () => {
      window.cancelAnimationFrame(frame);
      observer.disconnect();
    };
  }, [thread.title]);

  const titleStyle =
    titleOverflow.distance > 0
      ? ({
          "--thread-title-scroll-distance": `${titleOverflow.distance}px`,
          "--thread-title-scroll-duration": `${titleOverflow.durationMs}ms`,
        } as CSSProperties)
      : undefined;

  return (
    <div
      className={`thread-row-wrap ${active ? "active" : ""} ${
        isThreadActivityProcessing(activityStatus) ? "is-processing" : ""
      } ${activityStatus ? "has-status" : ""} ${menuOpen ? "menu-open" : ""}`}
    >
      <button
        className={`thread-row ${active ? "active" : ""}`}
        aria-current={active ? "page" : undefined}
        onPointerDown={(event) => {
          if (event.button === 0) onSelect(thread);
        }}
        onClick={(event) => {
          if (event.detail === 0) onSelect(thread);
        }}
        onContextMenu={(event) => {
          event.preventDefault();
          setMenuOpen(true);
        }}
        aria-label={thread.title}
        title={thread.title}
      >
        <span
          className={`thread-title-viewport ${titleOverflow.distance > 0 ? "is-overflowing" : ""}`}
          ref={titleViewportRef}
        >
          <span
            className="thread-title-text"
            ref={titleTextRef}
            style={titleStyle}
          >
            {thread.title}
          </span>
        </span>
      </button>
      <ThreadStatusIndicator status={activityStatus} />
      <div className="thread-row-menu-wrap" ref={menuRef}>
        <button
          className="thread-row-more"
          type="button"
          aria-label={`任务菜单 ${thread.title}`}
          aria-expanded={menuOpen}
          onClick={() => setMenuOpen((current) => !current)}
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
    </div>
  );
});
