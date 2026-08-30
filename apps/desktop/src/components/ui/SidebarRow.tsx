import {
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type MouseEventHandler,
  type ReactNode,
} from "react";
import { Circle, Loader2 } from "lucide-react";
import { threadTitleScrollDurationMs } from "../../threadTitleScroll";

export type SidebarRowStatusTone =
  "neutral" | "info" | "success" | "warning" | "danger";

export type SidebarRowStatus = {
  label: string;
  loading?: boolean;
  tone: SidebarRowStatusTone;
};

export type SidebarRowProps = {
  active?: boolean;
  actions?: ReactNode;
  actionsOpen?: boolean;
  ariaLabel?: string;
  className?: string;
  description?: string;
  onContextMenu?: MouseEventHandler<HTMLButtonElement>;
  onSelect?(): void;
  status?: SidebarRowStatus;
  title: string;
};

export function SidebarRow({
  active = false,
  actions,
  actionsOpen = false,
  ariaLabel,
  className,
  description,
  onContextMenu,
  onSelect,
  status,
  title,
}: SidebarRowProps) {
  const [titleOverflow, setTitleOverflow] = useState({
    distance: 0,
    durationMs: 0,
  });
  const titleViewportRef = useRef<HTMLSpanElement>(null);
  const titleTextRef = useRef<HTMLSpanElement>(null);

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
  }, [title]);

  const titleStyle =
    titleOverflow.distance > 0
      ? ({
          "--thread-title-scroll-distance": `${titleOverflow.distance}px`,
          "--thread-title-scroll-duration": `${titleOverflow.durationMs}ms`,
        } as CSSProperties)
      : undefined;
  const rowTitle = description ? `${title} · ${description}` : title;
  const titleContent = (
    <span
      className={`thread-title-viewport ${titleOverflow.distance > 0 ? "is-overflowing" : ""}`}
      ref={titleViewportRef}
    >
      <span className="thread-title-text" ref={titleTextRef} style={titleStyle}>
        {title}
      </span>
    </span>
  );

  return (
    <div
      className={[
        "thread-row-wrap",
        active ? "active" : "",
        status ? "has-status" : "",
        actions ? "has-actions" : "",
        actionsOpen ? "menu-open" : "",
        className ?? "",
      ]
        .filter(Boolean)
        .join(" ")}
    >
      {onSelect ? (
        <button
          aria-current={active ? "page" : undefined}
          aria-label={ariaLabel ?? title}
          className={`thread-row ${active ? "active" : ""}`}
          onClick={(event) => {
            if (event.detail === 0) onSelect();
          }}
          onContextMenu={onContextMenu}
          onPointerDown={(event) => {
            if (event.button === 0) onSelect();
          }}
          title={rowTitle}
          type="button"
        >
          {titleContent}
        </button>
      ) : (
        <div
          aria-label={ariaLabel}
          className={`thread-row ${active ? "active" : ""}`}
          title={rowTitle}
        >
          {titleContent}
        </div>
      )}
      {status ? <SidebarRowStatusIndicator status={status} /> : null}
      {actions}
    </div>
  );
}

function SidebarRowStatusIndicator({ status }: { status: SidebarRowStatus }) {
  return (
    <span
      aria-label={status.label}
      className={`thread-row-status is-${status.tone}`}
      role="img"
      title={status.label}
    >
      {status.loading ? (
        <Loader2
          aria-hidden="true"
          className="thread-status-spinner"
          size={14}
          strokeWidth={2.5}
        />
      ) : (
        <Circle aria-hidden="true" fill="currentColor" size={9} />
      )}
    </span>
  );
}
