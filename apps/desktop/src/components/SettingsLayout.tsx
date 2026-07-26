import type { ReactNode } from "react";

/**
 * Shared shell pieces for the settings surfaces.
 *
 * These live outside SettingsPanel so individual pages can import them without
 * a cycle back through the panel that renders them.
 */

export function SettingsPage({
  title,
  description,
  actions,
  children,
}: {
  title: string;
  description: string;
  actions?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="settings-page" aria-labelledby={`settings-${title}`}>
      <div className="settings-page-heading">
        <div>
          <h3 id={`settings-${title}`}>{title}</h3>
          <p>{description}</p>
        </div>
        {actions ? (
          <div className="settings-page-actions">{actions}</div>
        ) : null}
      </div>
      {children}
    </section>
  );
}

export function SettingsGroup({
  title,
  description,
  actions,
  children,
}: {
  title: string;
  description?: string;
  actions?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="settings-group">
      <div className="settings-group-heading">
        <div>
          <h4>{title}</h4>
          {description ? (
            <p className="settings-group-description">{description}</p>
          ) : null}
        </div>
        {actions ? (
          <div className="settings-group-actions-inline">{actions}</div>
        ) : null}
      </div>
      <div className="settings-group-body">{children}</div>
    </section>
  );
}

export function SettingsRow({
  title,
  description,
  control,
  disabled = false,
}: {
  title: string;
  description?: string;
  control?: ReactNode;
  disabled?: boolean;
}) {
  return (
    <div className={`settings-row ${disabled ? "disabled" : ""}`}>
      <div>
        <strong>{title}</strong>
        {description ? <span>{description}</span> : null}
      </div>
      {control ? <div className="settings-row-control">{control}</div> : null}
    </div>
  );
}
