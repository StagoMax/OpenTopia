import { ChevronDown } from "lucide-react";
import type { ReactNode, SelectHTMLAttributes } from "react";

export type SelectOption<T extends string> = {
  value: T;
  label: string;
  disabled?: boolean;
};

export type SelectProps<T extends string> = Omit<
  SelectHTMLAttributes<HTMLSelectElement>,
  "onChange" | "value" | "children"
> & {
  value: T;
  options: ReadonlyArray<SelectOption<T>>;
  onChange(value: T): void;
  /** Required when no visible <label> is wired to this control. */
  label?: string;
  /** Rendered inside the control, before the value. */
  leading?: ReactNode;
};

/**
 * A native select with the chevron drawn by us. Native keeps the popup, typing
 * to jump, and screen reader semantics correct on every platform, which a
 * div-based menu would have to reimplement.
 */
export function Select<T extends string>({
  className,
  label,
  leading,
  onChange,
  options,
  value,
  ...props
}: SelectProps<T>) {
  return (
    <span className={["ot-select", className ?? ""].filter(Boolean).join(" ")}>
      {leading ? (
        <span className="ot-select__leading" aria-hidden="true">
          {leading}
        </span>
      ) : null}
      <select
        className="ot-select__input"
        aria-label={label}
        value={value}
        onChange={(event) => onChange(event.target.value as T)}
        {...props}
      >
        {options.map((option) => (
          <option
            key={option.value}
            value={option.value}
            disabled={option.disabled}
          >
            {option.label}
          </option>
        ))}
      </select>
      <ChevronDown
        className="ot-select__chevron"
        size={14}
        aria-hidden="true"
        focusable="false"
      />
    </span>
  );
}
