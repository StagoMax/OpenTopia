import type { HTMLAttributes, ReactNode } from "react";

export type BadgeVariant =
  "neutral" | "info" | "success" | "warning" | "danger";

export type BadgeProps = HTMLAttributes<HTMLSpanElement> & {
  children: ReactNode;
  variant?: BadgeVariant;
};

export function Badge({
  children,
  className,
  variant = "neutral",
  ...props
}: BadgeProps) {
  const classes = ["ot-badge", `ot-badge--${variant}`, className ?? ""]
    .filter(Boolean)
    .join(" ");

  return (
    <span className={classes} {...props}>
      {children}
    </span>
  );
}
