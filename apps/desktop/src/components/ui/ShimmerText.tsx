import type { HTMLAttributes } from "react";

export type ShimmerTextProps = Omit<
  HTMLAttributes<HTMLSpanElement>,
  "children"
> & {
  children: string;
};

/**
 * A loading label with a compositor-friendly light sweep. The highlighted
 * copy counter-translates inside its moving mask so every glyph stays aligned
 * with the readable base text.
 */
export function ShimmerText({
  children,
  className,
  ...props
}: ShimmerTextProps) {
  const classes = ["ot-shimmer-text", className].filter(Boolean).join(" ");

  return (
    <span {...props} className={classes}>
      <span className="ot-shimmer-text__base">{children}</span>
      <span className="ot-shimmer-text__sweep" aria-hidden="true">
        <span className="ot-shimmer-text__highlight">{children}</span>
      </span>
    </span>
  );
}
