import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type FocusEventHandler,
  type PointerEventHandler,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";

import {
  calculateTooltipPosition,
  type TooltipPlacement,
  type TooltipPosition,
} from "./tooltipPosition";

export type TooltipTriggerProps = {
  "aria-describedby": string | undefined;
  onBlur: FocusEventHandler<HTMLButtonElement>;
  onFocus: FocusEventHandler<HTMLButtonElement>;
  onPointerEnter: PointerEventHandler<HTMLButtonElement>;
  onPointerLeave: PointerEventHandler<HTMLButtonElement>;
  onPointerMove: PointerEventHandler<HTMLButtonElement>;
  ref: (node: HTMLButtonElement | null) => void;
};

export type TooltipAnchor = "trigger" | "pointer";

export type TooltipProps = {
  children: (props: TooltipTriggerProps) => ReactNode;
  content: ReactNode;
  anchor?: TooltipAnchor;
  placement?: TooltipPlacement;
};

/**
 * Describes an interactive control on pointer hover and keyboard focus. The
 * surface is portalled so scrollable menus cannot clip longer descriptions.
 */
export function Tooltip({
  children,
  content,
  anchor = "trigger",
  placement = "top",
}: TooltipProps) {
  const [pointerInside, setPointerInside] = useState(false);
  const [focusInside, setFocusInside] = useState(false);
  const [position, setPosition] = useState<TooltipPosition | null>(null);
  const tooltipId = useId();
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const surfaceRef = useRef<HTMLDivElement | null>(null);
  const pointerPositionRef = useRef<{ x: number; y: number } | null>(null);
  const open = pointerInside || focusInside;

  const updatePosition = useCallback(() => {
    const triggerElement = triggerRef.current;
    const surfaceElement = surfaceRef.current;
    if (!triggerElement || !surfaceElement) return;

    const styles = getComputedStyle(document.documentElement);
    const gap = Number.parseFloat(styles.getPropertyValue("--space-2")) || 4;
    const margin =
      Number.parseFloat(styles.getPropertyValue("--space-4")) || gap * 2;
    const triggerRect = triggerElement.getBoundingClientRect();
    const pointerPosition = pointerPositionRef.current;
    const anchorRect =
      anchor === "pointer" && pointerInside && pointerPosition
        ? {
            top: pointerPosition.y,
            right: pointerPosition.x,
            bottom: pointerPosition.y,
            left: pointerPosition.x,
            width: 0,
            height: 0,
          }
        : triggerRect;
    setPosition(
      calculateTooltipPosition(
        anchorRect,
        surfaceElement.getBoundingClientRect(),
        { width: window.innerWidth, height: window.innerHeight },
        placement,
        gap,
        margin,
      ),
    );
  }, [anchor, placement, pointerInside]);

  useLayoutEffect(() => {
    if (!open) {
      setPosition(null);
      return;
    }
    updatePosition();
  }, [open, updatePosition]);

  useEffect(() => {
    if (!open) return undefined;

    function onKeyDown(event: KeyboardEvent) {
      if (event.key !== "Escape") return;
      setPointerInside(false);
      setFocusInside(false);
    }

    document.addEventListener("keydown", onKeyDown);
    window.addEventListener("resize", updatePosition);
    window.addEventListener("scroll", updatePosition, true);
    const resizeObserver = new ResizeObserver(updatePosition);
    if (triggerRef.current) resizeObserver.observe(triggerRef.current);
    if (surfaceRef.current) resizeObserver.observe(surfaceRef.current);
    return () => {
      document.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("resize", updatePosition);
      window.removeEventListener("scroll", updatePosition, true);
      resizeObserver.disconnect();
    };
  }, [open, updatePosition]);

  return (
    <>
      {children({
        "aria-describedby": open ? tooltipId : undefined,
        onBlur: () => setFocusInside(false),
        onFocus: () => setFocusInside(true),
        onPointerEnter: (event) => {
          pointerPositionRef.current = {
            x: event.clientX,
            y: event.clientY,
          };
          setPointerInside(true);
        },
        onPointerLeave: () => {
          pointerPositionRef.current = null;
          setPointerInside(false);
        },
        onPointerMove: (event) => {
          pointerPositionRef.current = {
            x: event.clientX,
            y: event.clientY,
          };
          if (pointerInside) updatePosition();
        },
        ref: (node) => {
          triggerRef.current = node;
        },
      })}
      {open
        ? createPortal(
            <div
              className="ot-tooltip-surface"
              data-placement={position?.placement ?? placement}
              id={tooltipId}
              ref={surfaceRef}
              role="tooltip"
              style={{
                left: position?.left ?? 0,
                top: position?.top ?? 0,
                visibility: position ? "visible" : "hidden",
              }}
            >
              {content}
            </div>,
            document.body,
          )
        : null}
    </>
  );
}
