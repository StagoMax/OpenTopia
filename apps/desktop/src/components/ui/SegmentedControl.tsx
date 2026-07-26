export type SegmentedOption<T extends string> = {
  value: T;
  label: string;
  disabled?: boolean;
};

export type SegmentedControlProps<T extends string> = {
  value: T;
  options: ReadonlyArray<SegmentedOption<T>>;
  onChange(value: T): void;
  /** Names the group for assistive tech; required, there is no visible label. */
  label: string;
  className?: string;
  disabled?: boolean;
};

/**
 * Mutually exclusive choice rendered inline. Uses radio semantics rather than a
 * row of buttons so the arrow keys move within the group and the selected
 * option is announced as such — selection is not signalled by color alone,
 * `aria-checked` carries it too.
 */
export function SegmentedControl<T extends string>({
  className,
  disabled = false,
  label,
  onChange,
  options,
  value,
}: SegmentedControlProps<T>) {
  return (
    <span
      className={["ot-segmented", className ?? ""].filter(Boolean).join(" ")}
      role="radiogroup"
      aria-label={label}
    >
      {options.map((option) => {
        const selected = option.value === value;
        return (
          <button
            key={option.value}
            type="button"
            role="radio"
            aria-checked={selected}
            className="ot-segmented__option"
            disabled={disabled || option.disabled}
            tabIndex={selected ? 0 : -1}
            onClick={() => onChange(option.value)}
            onKeyDown={(event) => {
              if (event.key !== "ArrowRight" && event.key !== "ArrowLeft")
                return;
              event.preventDefault();
              const usable = options.filter((item) => !item.disabled);
              const index = usable.findIndex((item) => item.value === value);
              if (index < 0) return;
              const step = event.key === "ArrowRight" ? 1 : -1;
              const next =
                usable[(index + step + usable.length) % usable.length];
              onChange(next.value);
            }}
          >
            {option.label}
          </button>
        );
      })}
    </span>
  );
}
