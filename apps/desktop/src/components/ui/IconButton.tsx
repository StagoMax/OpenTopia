import { forwardRef, type ButtonHTMLAttributes, type ReactNode } from "react";
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

export const IconButton = forwardRef<HTMLButtonElement, IconButtonProps>(
  function IconButton(
    { children, className, size = "default", variant = "quiet", ...props },
    ref,
  ) {
    const classes = [
      "ot-icon-button",
      size === "compact" ? "ot-icon-button--compact" : "",
      className ?? "",
    ]
      .filter(Boolean)
      .join(" ");

    return (
      <Button
        ref={ref}
        className={classes}
        size={size}
        variant={variant}
        {...props}
      >
        {children}
      </Button>
    );
  },
);
