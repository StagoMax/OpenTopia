import { useId, type InputHTMLAttributes, type ReactNode } from "react";

export type TextFieldProps = Omit<
  InputHTMLAttributes<HTMLInputElement>,
  "id"
> & {
  error?: ReactNode;
  hint?: ReactNode;
  id?: string;
  label: ReactNode;
  wrapperClassName?: string;
};

export function TextField({
  className,
  error,
  hint,
  id,
  label,
  wrapperClassName,
  ...props
}: TextFieldProps) {
  const generatedId = useId();
  const inputId = id ?? generatedId;
  const hintId = hint ? `${inputId}-hint` : undefined;
  const errorId = error ? `${inputId}-error` : undefined;
  const describedBy = [props["aria-describedby"], hintId, errorId]
    .filter(Boolean)
    .join(" ");
  const classes = ["ot-text-field__input", className ?? ""]
    .filter(Boolean)
    .join(" ");

  return (
    <label
      className={["ot-text-field", wrapperClassName ?? ""]
        .filter(Boolean)
        .join(" ")}
    >
      <span>{label}</span>
      <input
        {...props}
        aria-describedby={describedBy || undefined}
        aria-invalid={error ? true : props["aria-invalid"]}
        className={classes}
        id={inputId}
      />
      {hint ? (
        <span className="ot-text-field__hint" id={hintId}>
          {hint}
        </span>
      ) : null}
      {error ? (
        <span className="ot-text-field__error" id={errorId} role="alert">
          {error}
        </span>
      ) : null}
    </label>
  );
}
