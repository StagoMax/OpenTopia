import type { InputHTMLAttributes } from "react";

export type NumberFieldProps = Omit<
  InputHTMLAttributes<HTMLInputElement>,
  "onChange" | "type" | "value"
> & {
  value: number;
  onChange(value: number): void;
  /** Required when no visible <label> is wired to this control. */
  label?: string;
  /** Static unit rendered after the input, e.g. "px". */
  unit?: string;
};

/**
 * Numeric input with a trailing unit.
 *
 * Empty and mid-edit values are ignored rather than coerced to 0, so clearing
 * the field to retype a number does not momentarily apply a 0 to whatever the
 * value drives.
 */
export function NumberField({
  className,
  label,
  onChange,
  unit,
  value,
  ...props
}: NumberFieldProps) {
  return (
    <span
      className={["ot-number-field", className ?? ""].filter(Boolean).join(" ")}
    >
      <input
        className="ot-number-field__input"
        type="number"
        inputMode="numeric"
        aria-label={label}
        value={value}
        onChange={(event) => {
          const next = Number(event.target.value);
          if (event.target.value === "" || !Number.isFinite(next)) return;
          onChange(next);
        }}
        {...props}
      />
      {unit ? (
        <span className="ot-number-field__unit" aria-hidden="true">
          {unit}
        </span>
      ) : null}
    </span>
  );
}
