import {
  useEffect,
  useId,
  useRef,
  useState,
  type InputHTMLAttributes,
} from "react";
import { ChevronDown } from "lucide-react";

export type InputDropdownOption = {
  value: string;
  label: string;
  disabled?: boolean;
};

export type InputDropdownProps = {
  value: string | number;
  options: readonly InputDropdownOption[];
  selectedOptionValue?: string | null;
  inputProps?: Omit<
    InputHTMLAttributes<HTMLInputElement>,
    "className" | "value" | "onChange"
  >;
  label: string;
  menuLabel: string;
  onValueChange(value: string): void;
  onOptionSelect(value: string): void;
};

/**
 * A manual text field with a compact preset picker. The input stays editable;
 * selecting an option only fills or resets the value through the caller.
 */
export function InputDropdown({
  value,
  options,
  selectedOptionValue,
  inputProps,
  label,
  menuLabel,
  onValueChange,
  onOptionSelect,
}: InputDropdownProps) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const menuId = useId();
  const { disabled = false, onKeyDown, ...restInputProps } = inputProps ?? {};

  useEffect(() => {
    if (!open) return undefined;

    function onPointerDown(event: PointerEvent) {
      if (rootRef.current?.contains(event.target as Node)) return;
      setOpen(false);
    }

    function onKeyDownOutside(event: KeyboardEvent) {
      if (event.key === "Escape") setOpen(false);
    }

    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("keydown", onKeyDownOutside);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("keydown", onKeyDownOutside);
    };
  }, [open]);

  useEffect(() => {
    if (disabled) setOpen(false);
  }, [disabled]);

  return (
    <div className="ot-input-dropdown" ref={rootRef}>
      <input
        {...restInputProps}
        ref={inputRef}
        className="ot-input-dropdown__input"
        disabled={disabled}
        value={value}
        onChange={(event) => onValueChange(event.target.value)}
        onKeyDown={(event) => {
          onKeyDown?.(event);
          if (!event.defaultPrevented && event.key === "Escape") {
            setOpen(false);
          }
        }}
      />
      <button
        type="button"
        className={`ot-input-dropdown__trigger${open ? " is-open" : ""}`}
        aria-controls={menuId}
        aria-expanded={open}
        aria-haspopup="listbox"
        aria-label={label}
        title={label}
        disabled={disabled}
        onClick={() => setOpen((current) => !current)}
      >
        <ChevronDown size={16} aria-hidden="true" focusable="false" />
      </button>
      {open ? (
        <div
          id={menuId}
          className="ot-input-dropdown__menu"
          role="listbox"
          aria-label={menuLabel}
        >
          {options.map((option) => (
            <button
              key={option.value}
              type="button"
              className="ot-input-dropdown__option"
              role="option"
              aria-selected={option.value === selectedOptionValue}
              disabled={option.disabled}
              onClick={() => {
                onOptionSelect(option.value);
                setOpen(false);
                inputRef.current?.focus();
              }}
            >
              {option.label}
            </button>
          ))}
        </div>
      ) : null}
    </div>
  );
}
