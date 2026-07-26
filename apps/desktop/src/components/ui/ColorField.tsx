import { useEffect, useState } from "react";

export type ColorFieldProps = {
  value: string;
  onChange(value: string): void;
  /** Names both the swatch and the hex input for assistive tech. */
  label: string;
  className?: string;
  disabled?: boolean;
};

const SHORT_HEX = /^#[0-9a-fA-F]{3}$/;
const FULL_HEX = /^#[0-9a-fA-F]{6}$/;

function expandShortHex(value: string): string {
  const [, r, g, b] = value;
  return `#${r}${r}${g}${g}${b}${b}`.toUpperCase();
}

/**
 * A color swatch plus an editable hex field.
 *
 * The text input keeps its own draft state: typing a six-digit hex passes
 * through invalid prefixes on the way, and committing those would fight the
 * caret. The draft is only lifted when it parses, and it re-syncs when the
 * value changes from outside (theme switch, reset, import).
 */
export function ColorField({
  className,
  disabled = false,
  label,
  onChange,
  value,
}: ColorFieldProps) {
  const [draft, setDraft] = useState(value);

  useEffect(() => {
    setDraft(value);
  }, [value]);

  function commit(next: string) {
    const trimmed = next.trim();
    if (FULL_HEX.test(trimmed)) onChange(trimmed.toUpperCase());
    else if (SHORT_HEX.test(trimmed)) onChange(expandShortHex(trimmed));
    else setDraft(value);
  }

  const valid = FULL_HEX.test(draft.trim()) || SHORT_HEX.test(draft.trim());

  return (
    <span
      className={["ot-color-field", className ?? ""].filter(Boolean).join(" ")}
    >
      {FULL_HEX.test(value) ? (
        <input
          className="ot-color-field__swatch"
          type="color"
          aria-label={`${label}取色器`}
          disabled={disabled}
          value={value}
          onChange={(event) => onChange(event.target.value.toUpperCase())}
        />
      ) : (
        // No valid color to seed the native picker with; the hex field below is
        // the way out of this state, so the swatch is inert rather than absent.
        <span className="ot-color-field__swatch is-empty" aria-hidden="true" />
      )}
      <input
        className="ot-color-field__hex"
        type="text"
        spellCheck={false}
        autoComplete="off"
        aria-label={label}
        aria-invalid={valid ? undefined : true}
        disabled={disabled}
        value={draft}
        maxLength={7}
        onChange={(event) => setDraft(event.target.value)}
        onBlur={(event) => commit(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            commit(draft);
          }
          if (event.key === "Escape") setDraft(value);
        }}
      />
    </span>
  );
}
