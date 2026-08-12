import { forwardRef, type ButtonHTMLAttributes, type ReactNode } from "react";

export type ButtonVariant = "primary" | "secondary" | "quiet" | "danger";
export type ButtonSize = "compact" | "default";

export type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  children: ReactNode;
  size?: ButtonSize;
  variant?: ButtonVariant;
};

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(
  function Button(
    {
      children,
      className,
      size = "default",
      type = "button",
      variant = "secondary",
      ...props
    },
    ref,
  ) {
    const classes = [
      "ot-button",
      `ot-button--${variant}`,
      size === "compact" ? "ot-button--compact" : "",
      className ?? "",
    ]
      .filter(Boolean)
      .join(" ");

    return (
      <button ref={ref} className={classes} type={type} {...props}>
        {children}
      </button>
    );
  },
);
