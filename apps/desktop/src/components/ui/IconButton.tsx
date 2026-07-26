import type { ButtonHTMLAttributes, ReactNode } from "react";
import { Button, type ButtonSize, type ButtonVariant } from "./Button";

export type IconButtonProps = Omit<
  ButtonHTMLAttributes<HTMLButtonElement>,
  "children"
> & {
  "aria-label": string;
  children: ReactNode;
  size?: ButtonSize;
  variant?: ButtonVariant;
};

export function IconButton({
  children,
  className,
  size = "default",
  variant = "quiet",
  ...props
}: IconButtonProps) {
  const classes = [
    "ot-icon-button",
    size === "compact" ? "ot-icon-button--compact" : "",
    className ?? "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <Button className={classes} size={size} variant={variant} {...props}>
      {children}
    </Button>
  );
}
