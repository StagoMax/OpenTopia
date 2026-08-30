import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";

export type PopoverAlign = "start" | "end";
export type PopoverPlacement = "top" | "bottom";

export type PopoverProps = {
  /**
   * Renders the control that opens the popover. `props` must be spread onto a
   * native `button` so keyboard and accessibility wiring stays intact.
   */
  trigger: (props: {
    "aria-controls": string;
    "aria-expanded": boolean;
    "aria-haspopup": "dialog";
    onClick: () => void;
    ref: (node: HTMLButtonElement | null) => void;
  }) => ReactNode;
  children: (props: { close: () => void }) => ReactNode;
  align?: PopoverAlign;
  autoFocus?: boolean;
  placement?: PopoverPlacement;
  /** Accessible name for the floating surface. */
  label: string;
  /** Optional controlled open state for keyboard-driven feature commands. */
  open?: boolean;
  onOpenChange?(open: boolean): void;
};

type PopoverPosition = { left: number; top: number };

/**
 * A floating surface anchored above its trigger. Closes on outside click and
 * on Escape, and returns focus to the trigger so keyboard users are not
 * stranded.
 */
export function Popover({
  trigger,
  children,
  align = "start",
  autoFocus = false,
  placement = "top",
  label,
  open: controlledOpen,
  onOpenChange,
}: PopoverProps) {
  const [uncontrolledOpen, setUncontrolledOpen] = useState(false);
  const open = controlledOpen ?? uncontrolledOpen;
  const [position, setPosition] = useState<PopoverPosition | null>(null);
  const surfaceId = useId();
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const surfaceRef = useRef<HTMLDivElement | null>(null);

  const setOpen = useCallback(
    (nextOpen: boolean) => {
      if (controlledOpen === undefined) setUncontrolledOpen(nextOpen);
      onOpenChange?.(nextOpen);
    },
    [controlledOpen, onOpenChange],
  );

  const close = useCallback(() => {
    setOpen(false);
    setPosition(null);
    triggerRef.current?.focus();
  }, [setOpen]);

  const updatePosition = useCallback(() => {
    const triggerElement = triggerRef.current;
    const surfaceElement = surfaceRef.current;
    if (!triggerElement || !surfaceElement) return;

    const triggerRect = triggerElement.getBoundingClientRect();
    const surfaceRect = surfaceElement.getBoundingClientRect();
    const styles = getComputedStyle(document.documentElement);
    const gap = Number.parseFloat(styles.getPropertyValue("--space-2")) || 4;
    const margin =
      Number.parseFloat(styles.getPropertyValue("--space-4")) || gap * 2;
    const below = triggerRect.bottom + gap;
    const above = triggerRect.top - surfaceRect.height - gap;
    const bottomLimit = window.innerHeight - margin;
    let top = placement === "bottom" ? below : above;

    if (placement === "bottom" && top + surfaceRect.height > bottomLimit) {
      top = above >= margin ? above : top;
    } else if (placement === "top" && top < margin) {
      top = below + surfaceRect.height <= bottomLimit ? below : top;
    }

    const desiredLeft =
      align === "end"
        ? triggerRect.right - surfaceRect.width
        : triggerRect.left;
    const left = Math.min(
      Math.max(margin, desiredLeft),
      Math.max(margin, window.innerWidth - surfaceRect.width - margin),
    );

    setPosition({
      left,
      top: Math.min(
        Math.max(margin, top),
        Math.max(margin, bottomLimit - surfaceRect.height),
      ),
    });
  }, [align, placement]);

  useLayoutEffect(() => {
    if (!open) return;
    updatePosition();
  }, [open, updatePosition]);

  useEffect(() => {
    if (!open || !autoFocus) return undefined;
    const frame = window.requestAnimationFrame(() => {
      surfaceRef.current
        ?.querySelector<HTMLElement>(
          "button:not(:disabled), [href], input:not(:disabled), [tabindex]:not([tabindex='-1'])",
        )
        ?.focus();
    });
    return () => window.cancelAnimationFrame(frame);
  }, [autoFocus, open, updatePosition]);

  useEffect(() => {
    if (!open) return undefined;

    function onPointerDown(event: PointerEvent) {
      const target = event.target as Node;
      if (surfaceRef.current?.contains(target)) return;
      if (triggerRef.current?.contains(target)) return;
      setOpen(false);
    }

    function onKeyDown(event: KeyboardEvent) {
      if (event.key !== "Escape") return;
      event.stopPropagation();
      close();
    }

    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    window.addEventListener("resize", updatePosition);
    window.addEventListener("scroll", updatePosition, true);
    const resizeObserver = new ResizeObserver(updatePosition);
    if (triggerRef.current) resizeObserver.observe(triggerRef.current);
    if (surfaceRef.current) resizeObserver.observe(surfaceRef.current);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("resize", updatePosition);
      window.removeEventListener("scroll", updatePosition, true);
      resizeObserver.disconnect();
    };
  }, [close, open, updatePosition]);

  return (
    <div className="ot-popover">
      {trigger({
        "aria-controls": surfaceId,
        "aria-expanded": open,
        "aria-haspopup": "dialog",
        onClick: () => setOpen(!open),
        ref: (node) => {
          triggerRef.current = node;
        },
      })}
      {open
        ? createPortal(
            <div
              aria-label={label}
              className={`ot-popover-surface ot-popover-surface--${align}`}
              id={surfaceId}
              ref={surfaceRef}
              role="dialog"
              style={{
                left: position?.left ?? 0,
                top: position?.top ?? 0,
                visibility: position ? "visible" : "hidden",
              }}
            >
              {children({ close })}
            </div>,
            document.body,
          )
        : null}
    </div>
  );
}
