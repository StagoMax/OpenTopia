import { AlertTriangle, Cable, Save } from "lucide-react";
import { useEffect, useMemo, useState, type FormEvent } from "react";
import { useApplicationLanguage } from "../../ApplicationLanguageProvider";
import type { Connection, McpServerView } from "../../types";
import { Button, SelectField, Switch, TextField } from "../ui";
import {
  connectionFormFromConnection,
  connectionInputFromForm,
  emptyConnectionForm,
  ownerTypeLabel,
  validateConnectionForm,
  type ConnectionFormErrors,
  type ConnectionFormValues,
} from "./model";
import type { ConnectionsSnapshot, ConnectionsStore } from "./store";

export type ConnectionEditorProps = {
  formId?: string;
  submitAction?: "inline" | "external";
  snapshot: ConnectionsSnapshot;
  store: ConnectionsStore;
};

export function ConnectionEditor({
  formId,
  snapshot,
  store,
  submitAction = "inline",
}: ConnectionEditorProps) {
  const { language, t } = useApplicationLanguage();
  const editing = snapshot.connections.find(
    (connection) => connection.id === snapshot.selectedConnectionId,
  );
  const initialValues = useMemo(
    () =>
      snapshot.editorMode === "edit" && editing
        ? connectionFormFromConnection(editing)
        : emptyConnectionForm(snapshot.definitions),
    [editing, snapshot.definitions, snapshot.editorMode],
  );
  const [values, setValues] = useState(initialValues);
  const [errors, setErrors] = useState<ConnectionFormErrors>({});

  useEffect(() => {
    setValues(initialValues);
    setErrors({});
  }, [initialValues]);

  const reservedServerIds = useMemo(
    () =>
      new Set(
        snapshot.connections
          .filter((connection) => connection.id !== editing?.id)
          .map((connection) => connection.runtimeBinding.serverId),
      ),
    [editing?.id, snapshot.connections],
  );
  const saving =
    snapshot.busyAction?.startsWith("save:") ||
    snapshot.busyAction === "create";

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const nextErrors = validateConnectionForm(
      values,
      reservedServerIds,
      language,
    );
    setErrors(nextErrors);
    if (Object.keys(nextErrors).length > 0) return;
    await store.save(connectionInputFromForm(values));
  }

  return (
    <form
      className="connections-editor"
      id={formId}
      onSubmit={(event) => void submit(event)}
    >
      <header className="connections-editor__header">
        <span className="connections-icon connections-icon--large">
          <Cable aria-hidden="true" size={18} />
        </span>
        <span>
          <h2>{t("flow.connection.editor.heading")}</h2>
          <p>{t("flow.connection.editor.headingDetail")}</p>
        </span>
      </header>

      {snapshot.error ? (
        <div
          className="connections-feedback connections-feedback--error"
          role="alert"
        >
          <AlertTriangle aria-hidden="true" size={16} />
          <span>{snapshot.error}</span>
        </div>
      ) : null}

      <section
        className="connections-form-section"
        aria-labelledby="connection-provider-title"
      >
        <header>
          <h3 id="connection-provider-title">
            {t("flow.connection.editor.providerRuntime")}
          </h3>
          <p>{t("flow.connection.editor.providerRuntimeDetail")}</p>
        </header>
        <div className="connections-form-grid">
          <LabeledSelect
            disabled={snapshot.editorMode === "edit"}
            error={errors.definition}
            label={
              snapshot.editorMode === "edit"
                ? t("flow.connection.editor.providerDefinitionLocked")
                : t("flow.connection.editor.providerDefinition")
            }
            options={snapshot.definitions.map((definition) => ({
              value: definition.id,
              label:
                definition.kind === "mcp"
                  ? definition.name
                  : `${definition.name}（${t("flow.connection.editor.later")}）`,
              disabled: definition.kind !== "mcp" || !definition.enabled,
            }))}
            value={values.integrationDefinitionId}
            onChange={(integrationDefinitionId) =>
              setValues((current) => ({ ...current, integrationDefinitionId }))
            }
          />
          <LabeledSelect
            error={errors.server}
            label={t("flow.connection.editor.mcpRuntime")}
            options={runtimeOptions(
              snapshot.mcpServers,
              reservedServerIds,
              editing,
              t("flow.connection.editor.bound"),
            )}
            value={values.serverId}
            onChange={(serverId) =>
              setValues((current) => ({ ...current, serverId }))
            }
          />
          <TextField
            error={errors.name}
            label={t("flow.connection.editor.name")}
            placeholder={t("flow.connection.editor.namePlaceholder")}
            value={values.name}
            onChange={(event) =>
              setValues((current) => ({ ...current, name: event.target.value }))
            }
          />
          <TextField
            error={errors.environment}
            hint={t("flow.connection.editor.environmentHint")}
            label={t("flow.connection.editor.environment")}
            value={values.environment}
            onChange={(event) =>
              setValues((current) => ({
                ...current,
                environment: event.target.value,
              }))
            }
          />
        </div>
        <p className="connections-form-note">
          {t("flow.connection.editor.runtimeNote")}
        </p>
      </section>

      <section
        className="connections-form-section"
        aria-labelledby="connection-account-title"
      >
        <header>
          <h3 id="connection-account-title">
            {t("flow.connection.editor.accountTenant")}
          </h3>
          <p>{t("flow.connection.editor.accountTenantDetail")}</p>
        </header>
        <div className="connections-form-grid">
          <LabeledSelect
            label={t("flow.connection.editor.accountType")}
            options={(
              ["personal", "org_shared", "service_account"] as const
            ).map((ownerType) => ({
              value: ownerType,
              label: ownerTypeLabel(ownerType, language),
            }))}
            value={values.ownerType}
            onChange={(ownerType) =>
              setValues((current) => ({ ...current, ownerType }))
            }
          />
          <TextField
            label={t("flow.connection.editor.accountDisplayName")}
            placeholder={t(
              "flow.connection.editor.accountDisplayNamePlaceholder",
            )}
            value={values.accountDisplayName}
            onChange={(event) =>
              setValues((current) => ({
                ...current,
                accountDisplayName: event.target.value,
              }))
            }
          />
          <TextField
            label={t("flow.connection.editor.externalAccountId")}
            value={values.externalAccountId}
            onChange={(event) =>
              setValues((current) => ({
                ...current,
                externalAccountId: event.target.value,
              }))
            }
          />
          <TextField
            label={t("flow.connection.editor.credentialReference")}
            hint={t("flow.connection.editor.credentialHint")}
            placeholder={t("flow.connection.editor.credentialPlaceholder")}
            value={values.credentialRef}
            onChange={(event) =>
              setValues((current) => ({
                ...current,
                credentialRef: event.target.value,
              }))
            }
          />
          <TextField
            label={t("flow.connection.editor.tenantName")}
            value={values.tenantName}
            onChange={(event) =>
              setValues((current) => ({
                ...current,
                tenantName: event.target.value,
              }))
            }
          />
          <TextField
            label={t("flow.connection.editor.tenantId")}
            value={values.tenantId}
            onChange={(event) =>
              setValues((current) => ({
                ...current,
                tenantId: event.target.value,
              }))
            }
          />
          <TextField
            label={t("flow.connection.editor.workspaceName")}
            value={values.workspaceName}
            onChange={(event) =>
              setValues((current) => ({
                ...current,
                workspaceName: event.target.value,
              }))
            }
          />
          <TextField
            label={t("flow.connection.editor.workspaceId")}
            value={values.workspaceId}
            onChange={(event) =>
              setValues((current) => ({
                ...current,
                workspaceId: event.target.value,
              }))
            }
          />
        </div>
        <TextField
          hint={t("flow.connection.editor.grantedScopesHint")}
          label={t("flow.connection.editor.grantedScopes")}
          placeholder="crm.read, deals.write"
          value={values.grantedScopes}
          onChange={(event) =>
            setValues((current) => ({
              ...current,
              grantedScopes: event.target.value,
            }))
          }
        />
      </section>

      <footer className="connections-editor__footer">
        <label className="connections-switch-field">
          <Switch
            checked={values.enabled}
            label={t("flow.connection.editor.enabled")}
            onChange={(enabled) =>
              setValues((current) => ({ ...current, enabled }))
            }
          />
          <span>
            <strong>{t("flow.connection.editor.enabled")}</strong>
            <small>{t("flow.connection.editor.enabledDetail")}</small>
          </span>
        </label>
        {submitAction === "inline" ? (
          <Button disabled={Boolean(saving)} type="submit" variant="primary">
            <Save aria-hidden="true" size={14} />
            {saving
              ? t("flow.connection.saving")
              : t("flow.connection.editor.saveConnection")}
          </Button>
        ) : null}
      </footer>
    </form>
  );
}

function LabeledSelect<T extends string>({
  disabled,
  error,
  label,
  onChange,
  options,
  value,
}: {
  disabled?: boolean;
  error?: string;
  label: string;
  onChange(value: T): void;
  options: ReadonlyArray<{ value: T; label: string; disabled?: boolean }>;
  value: T;
}) {
  const { t } = useApplicationLanguage();
  return (
    <SelectField
      fieldClassName="connections-select-field"
      aria-invalid={error ? true : undefined}
      disabled={disabled}
      error={error}
      label={label}
      onChange={onChange}
      options={
        options.length > 0
          ? options
          : [
              {
                value: "" as T,
                label: t("flow.connection.editor.noOptions"),
                disabled: true,
              },
            ]
      }
      value={value}
    />
  );
}

function runtimeOptions(
  servers: readonly McpServerView[],
  reservedServerIds: ReadonlySet<string>,
  editing: Connection | undefined,
  boundLabel: string,
) {
  return servers.map(({ server, status }) => {
    const reserved =
      reservedServerIds.has(server.serverId) &&
      server.serverId !== editing?.runtimeBinding.serverId;
    return {
      value: server.serverId,
      label: `${server.name} · ${status.status}${reserved ? ` · ${boundLabel}` : ""}`,
      disabled: reserved,
    };
  });
}
