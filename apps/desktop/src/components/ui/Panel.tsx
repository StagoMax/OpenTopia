import type { HTMLAttributes, ReactNode } from "react";

export type PanelProps = HTMLAttributes<HTMLElement> & {
  actions?: ReactNode;
  children: ReactNode;
  title?: ReactNode;
};

export function Panel({
  actions,
  children,
  className,
  title,
  ...props
}: PanelProps) {
  const classes = ["ot-panel", className ?? ""].filter(Boolean).join(" ");

  return (
    <section className={classes} {...props}>
      {title ? (
        <header className="ot-panel__header">
          <h2 className="ot-panel__title">{title}</h2>
          {actions}
        </header>
      ) : null}
      <div className="ot-panel__body">{children}</div>
    </section>
  );
}
