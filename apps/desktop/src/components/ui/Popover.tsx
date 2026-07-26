import {
  useCallback,
  useEffect,
  useId,
  useRef,
  useState,
  type ReactNode,
} from "react";

export type PopoverAlign = "start" | "end";

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
  /** Accessible name for the floating surface. */
  label: string;
};

/**
 * A floating surface anchored above its trigger. Closes on outside click and
 * on Escape, and returns focus to the trigger so keyboard users are not
 * stranded.
 */
export function Popover({
  trigger,
  children,
  align = "start",
  label,
}: PopoverProps) {
  const [open, setOpen] = useState(false);
  const surfaceId = useId();
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const surfaceRef = useRef<HTMLDivElement | null>(null);

  const close = useCallback(() => {
    setOpen(false);
    triggerRef.current?.focus();
  }, []);

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
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [close, open]);

  return (
    <div className="ot-popover">
      {trigger({
        "aria-controls": surfaceId,
        "aria-expanded": open,
        "aria-haspopup": "dialog",
        onClick: () => setOpen((value) => !value),
        ref: (node) => {
          triggerRef.current = node;
        },
      })}
      {open ? (
        <div
          aria-label={label}
          className={`ot-popover-surface ot-popover-surface--${align}`}
          id={surfaceId}
          ref={surfaceRef}
          role="dialog"
        >
          {children({ close })}
        </div>
      ) : null}
    </div>
  );
}
