import type { ReactNode } from "react";
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
  return (
    <section className="flow-workspace-inspector" aria-label={title}>
      <header className="flow-workspace-inspector__header">
        <span className="flow-workspace-inspector__heading">
          <strong>{title}</strong>
          {subtitle ? <small>{subtitle}</small> : null}
        </span>
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
  return (
    <section className="flow-workspace-inspector__section">
      <header>
        <strong>{title}</strong>
      </header>
      {children}
    </section>
  );
}
