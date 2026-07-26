import type { InputHTMLAttributes } from "react";

export type SliderProps = Omit<
  InputHTMLAttributes<HTMLInputElement>,
  "onChange" | "type" | "value"
> & {
  value: number;
  onChange(value: number): void;
  /** Required when no visible <label> is wired to this control. */
  label?: string;
  /** Shows the numeric value beside the track. */
  showValue?: boolean;
};

/**
 * Native range input. The value is rendered as text next to it so the setting
 * is never communicated by thumb position alone.
 */
export function Slider({
  className,
  label,
  max = 100,
  min = 0,
  onChange,
  showValue = true,
  step = 1,
  value,
  ...props
}: SliderProps) {
  const numericMin = Number(min);
  const numericMax = Number(max);
  const ratio =
    numericMax === numericMin
      ? 0
      : (value - numericMin) / (numericMax - numericMin);

  return (
    <span className={["ot-slider", className ?? ""].filter(Boolean).join(" ")}>
      <input
        className="ot-slider__input"
        type="range"
        aria-label={label}
        min={min}
        max={max}
        step={step}
        value={value}
        style={
          {
            "--ot-slider-fill": `${Math.round(ratio * 100)}%`,
          } as React.CSSProperties
        }
        onChange={(event) => onChange(Number(event.target.value))}
        {...props}
      />
      {showValue ? <span className="ot-slider__value">{value}</span> : null}
    </span>
  );
}
