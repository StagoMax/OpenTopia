import { Check, ChevronDown } from "lucide-react";
import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type ButtonHTMLAttributes,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";

import {
  firstEnabledOptionIndex,
  lastEnabledOptionIndex,
  moveEnabledOptionIndex,
  selectedOrFirstEnabledOptionIndex,
} from "./selectNavigation";

export type SelectOption<T extends string> = {
  value: T;
  label: string;
  disabled?: boolean;
};

export type SelectProps<T extends string> = Omit<
  ButtonHTMLAttributes<HTMLButtonElement>,
  "children" | "form" | "name" | "onChange" | "type" | "value"
> & {
  value: T;
  options: ReadonlyArray<SelectOption<T>>;
  onChange(value: T): void;
  /** Required when no visible <label> is wired to this control. */
  label?: string;
  /** Rendered inside the control, before the value. */
  leading?: ReactNode;
  /** Preserves native form serialization without exposing a system picker. */
  name?: string;
  form?: string;
  required?: boolean;
};

type SelectMenuPosition = {
  left: number;
  top: number;
  width: number;
};

/**
 * A consistent, app-owned menu select. Using a floating menu instead of the
 * Electron system picker keeps options visually integrated with the form while
 * retaining button semantics and full keyboard navigation.
 */
export function Select<T extends string>({
  className,
  disabled = false,
  form,
  label,
  leading,
  name,
  onChange,
  onClick,
  onKeyDown,
  options,
  required = false,
  value,
  ...props
}: SelectProps<T>) {
  const [open, setOpen] = useState(false);
  const [activeOptionIndex, setActiveOptionIndex] = useState(() =>
    selectedOrFirstEnabledOptionIndex(options, value),
  );
  const [position, setPosition] = useState<SelectMenuPosition | null>(null);
  const menuId = useId();
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);
  const optionRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const selectedOption = options.find((option) => option.value === value);
  const selectedOptionIndex = options.findIndex(
    (option) => option.value === value,
  );
  const { "aria-labelledby": labelledBy, ...buttonProps } = props;

  const close = useCallback((restoreFocus: boolean) => {
    setOpen(false);
    setPosition(null);
    if (restoreFocus) triggerRef.current?.focus();
  }, []);

  const updatePosition = useCallback(() => {
    const trigger = triggerRef.current;
    const menu = menuRef.current;
    if (!trigger || !menu) return;

    const styles = getComputedStyle(document.documentElement);
    const gap = Number.parseFloat(styles.getPropertyValue("--space-2")) || 4;
    const margin =
      Number.parseFloat(styles.getPropertyValue("--space-4")) || gap * 2;
    const triggerRect = trigger.getBoundingClientRect();
    const menuRect = menu.getBoundingClientRect();
    const maxLeft = Math.max(
      margin,
      window.innerWidth - triggerRect.width - margin,
    );
    const left = Math.min(Math.max(margin, triggerRect.left), maxLeft);
    const below = triggerRect.bottom + gap;
    const above = triggerRect.top - menuRect.height - gap;
    const bottomLimit = window.innerHeight - margin;
    const top =
      below + menuRect.height <= bottomLimit || above < margin
        ? Math.min(below, Math.max(margin, bottomLimit - menuRect.height))
        : above;

    setPosition({ left, top, width: triggerRect.width });
  }, []);

  const openMenu = useCallback(() => {
    const initialOptionIndex = selectedOrFirstEnabledOptionIndex(
      options,
      value,
    );
    if (disabled || initialOptionIndex < 0) return;
    setActiveOptionIndex(initialOptionIndex);
    setOpen(true);
  }, [disabled, options, value]);

  const moveActiveOption = useCallback(
    (direction: 1 | -1) => {
      setActiveOptionIndex((currentIndex) =>
        moveEnabledOptionIndex(options, currentIndex, direction),
      );
    },
    [options],
  );

  const selectOption = useCallback(
    (option: SelectOption<T>) => {
      if (option.disabled) return;
      onChange(option.value);
      close(true);
    },
    [close, onChange],
  );

  useLayoutEffect(() => {
    if (!open) return;
    updatePosition();
  }, [open, updatePosition]);

  useEffect(() => {
    if (!open || activeOptionIndex < 0) return undefined;
    const frame = requestAnimationFrame(() => {
      optionRefs.current[activeOptionIndex]?.focus();
    });
    return () => cancelAnimationFrame(frame);
  }, [activeOptionIndex, open]);

  useEffect(() => {
    if (!open) return undefined;

    function onPointerDown(event: PointerEvent) {
      const target = event.target as Node;
      if (
        triggerRef.current?.contains(target) ||
        menuRef.current?.contains(target)
      ) {
        return;
      }
      close(false);
    }

    function onWindowChange() {
      updatePosition();
    }

    document.addEventListener("pointerdown", onPointerDown);
    window.addEventListener("resize", onWindowChange);
    window.addEventListener("scroll", onWindowChange, true);
    const resizeObserver = new ResizeObserver(updatePosition);
    if (triggerRef.current) resizeObserver.observe(triggerRef.current);
    if (menuRef.current) resizeObserver.observe(menuRef.current);

    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      window.removeEventListener("resize", onWindowChange);
      window.removeEventListener("scroll", onWindowChange, true);
      resizeObserver.disconnect();
    };
  }, [close, open, updatePosition]);

  function handleTriggerKeyDown(event: KeyboardEvent<HTMLButtonElement>) {
    onKeyDown?.(event);
    if (event.defaultPrevented) return;

    if (
      event.key === "ArrowDown" ||
      event.key === "ArrowUp" ||
      event.key === " "
    ) {
      event.preventDefault();
      openMenu();
    } else if (event.key === "Escape" && open) {
      event.preventDefault();
      close(false);
    }
  }

  function handleOptionKeyDown(
    event: KeyboardEvent<HTMLButtonElement>,
    index: number,
  ) {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      moveActiveOption(1);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      moveActiveOption(-1);
    } else if (event.key === "Home") {
      event.preventDefault();
      setActiveOptionIndex(firstEnabledOptionIndex(options));
    } else if (event.key === "End") {
      event.preventDefault();
      setActiveOptionIndex(lastEnabledOptionIndex(options));
    } else if (event.key === "Escape") {
      event.preventDefault();
      close(true);
    } else if (event.key === "Tab") {
      close(false);
    } else if (index !== activeOptionIndex) {
      setActiveOptionIndex(index);
    }
  }

  return (
    <span
      className={["ot-select", open ? "ot-select--open" : "", className ?? ""]
        .filter(Boolean)
        .join(" ")}
    >
      {name ? (
        <input
          form={form}
          name={name}
          tabIndex={-1}
          type="hidden"
          value={value}
        />
      ) : null}
      {leading ? (
        <span className="ot-select__leading" aria-hidden="true">
          {leading}
        </span>
      ) : null}
      <button
        {...buttonProps}
        ref={triggerRef}
        aria-controls={open ? menuId : undefined}
        aria-expanded={open}
        aria-haspopup="menu"
        aria-label={labelledBy ? undefined : label}
        aria-labelledby={labelledBy}
        aria-required={required || undefined}
        className="ot-select__trigger"
        disabled={disabled}
        onClick={(event) => {
          onClick?.(event);
          if (!event.defaultPrevented) open ? close(false) : openMenu();
        }}
        onKeyDown={handleTriggerKeyDown}
        type="button"
      >
        <span className="ot-select__value">
          {selectedOption?.label ?? value}
        </span>
      </button>
      <ChevronDown
        className="ot-select__chevron"
        size={14}
        aria-hidden="true"
        focusable="false"
      />
      {open
        ? createPortal(
            <div
              aria-label={labelledBy ? undefined : label}
              aria-labelledby={labelledBy}
              className="ot-select__menu"
              id={menuId}
              ref={menuRef}
              role="menu"
              style={{
                left: position?.left ?? 0,
                top: position?.top ?? 0,
                visibility: position ? "visible" : "hidden",
                width:
                  position?.width ??
                  triggerRef.current?.getBoundingClientRect().width,
              }}
            >
              {options.map((option, index) => (
                <button
                  key={option.value}
                  ref={(node) => {
                    optionRefs.current[index] = node;
                  }}
                  aria-checked={index === selectedOptionIndex}
                  className="ot-select__option"
                  disabled={option.disabled}
                  onClick={() => selectOption(option)}
                  onFocus={() => setActiveOptionIndex(index)}
                  onKeyDown={(event) => handleOptionKeyDown(event, index)}
                  role="menuitemradio"
                  type="button"
                >
                  <span>{option.label}</span>
                  {index === selectedOptionIndex ? (
                    <Check aria-hidden="true" focusable="false" size={14} />
                  ) : null}
                </button>
              ))}
            </div>,
            document.body,
          )
        : null}
    </span>
  );
}
