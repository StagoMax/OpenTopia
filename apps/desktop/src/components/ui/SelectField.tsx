import { useId, type ReactNode } from "react";

import { Select, type SelectOption, type SelectProps } from "./Select";

export type SelectFieldProps<T extends string> = Omit<
  SelectProps<T>,
  "label" | "options"
> & {
  label: ReactNode;
  options: ReadonlyArray<SelectOption<T>>;
  hint?: ReactNode;
  error?: ReactNode;
  fieldClassName?: string;
};

/**
 * A complete select field with one visual control boundary. Keeping the label,
 * help text, error state, and app-owned menu in one primitive prevents feature
 * forms from wrapping an already bordered Select in another control shell.
 */
export function SelectField<T extends string>({
  error,
  fieldClassName,
  hint,
  id: providedId,
  label,
  options,
  ...props
}: SelectFieldProps<T>) {
  const generatedId = useId();
  const id = providedId ?? generatedId;
  const labelId = `${id}-label`;
  const supportingTextId = hint || error ? `${id}-description` : undefined;
  const describedBy =
    [props["aria-describedby"], supportingTextId].filter(Boolean).join(" ") ||
    undefined;

  return (
    <label
      className={["ot-select-field", fieldClassName ?? ""]
        .filter(Boolean)
        .join(" ")}
      htmlFor={id}
    >
      <span className="ot-select-field__label" id={labelId}>
        {label}
      </span>
      <Select
        {...props}
        aria-describedby={describedBy}
        aria-invalid={error ? true : props["aria-invalid"]}
        aria-labelledby={[props["aria-labelledby"], labelId]
          .filter(Boolean)
          .join(" ")}
        id={id}
        options={options}
      />
      {error ? (
        <small
          className="ot-select-field__error"
          id={supportingTextId}
          role="alert"
        >
          {error}
        </small>
      ) : hint ? (
        <small className="ot-select-field__hint" id={supportingTextId}>
          {hint}
        </small>
      ) : null}
    </label>
  );
}
