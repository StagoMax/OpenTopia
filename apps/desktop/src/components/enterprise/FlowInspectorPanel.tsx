import { useId, type ReactNode } from "react";
import { Badge } from "../ui";

export type FlowInspectorStatusVariant =
  "neutral" | "info" | "success" | "warning" | "danger";

export function FlowInspectorPanel({
  actions,
  children,
  status,
  statusVariant = "neutral",
  subtitle,
  title,
}: {
  actions?: ReactNode;
  children: ReactNode;
  status?: string;
  statusVariant?: FlowInspectorStatusVariant;
  subtitle?: string;
  title: string;
}) {
  const headingId = useId();

  return (
    <section className="flow-workspace-inspector" aria-labelledby={headingId}>
      <header className="flow-workspace-inspector__header">
        <div className="flow-workspace-inspector__heading">
          <h2 id={headingId}>{title}</h2>
          {subtitle ? <small>{subtitle}</small> : null}
        </div>
        <span className="flow-workspace-inspector__header-actions">
          {status ? <Badge variant={statusVariant}>{status}</Badge> : null}
          {actions}
        </span>
      </header>
      <div className="flow-workspace-inspector__body">{children}</div>
    </section>
  );
}

export function FlowInspectorSection({
  children,
  title,
}: {
  children: ReactNode;
  title: string;
}) {
  const headingId = useId();

  return (
    <section
      className="flow-workspace-inspector__section"
      aria-labelledby={headingId}
    >
      <header>
        <h3 id={headingId}>{title}</h3>
      </header>
      {children}
    </section>
  );
}
