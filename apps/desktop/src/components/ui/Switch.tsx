import type { ButtonHTMLAttributes } from "react";

export type SwitchProps = Omit<
  ButtonHTMLAttributes<HTMLButtonElement>,
  "onChange" | "role" | "aria-checked" | "children"
> & {
  checked: boolean;
  onChange(checked: boolean): void;
  /** Required when the switch has no visible <label> pointing at it. */
  label?: string;
};

/**
 * A native button carrying `role="switch"`, so keyboard activation and screen
 * reader state come from the platform rather than hand-rolled key handling.
 */
export function Switch({
  checked,
  className,
  disabled,
  label,
  onChange,
  ...props
}: SwitchProps) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      className={["ot-switch", className ?? ""].filter(Boolean).join(" ")}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      {...props}
    >
      <span className="ot-switch__knob" />
    </button>
  );
}
