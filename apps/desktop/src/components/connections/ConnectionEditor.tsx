import { AlertTriangle, Cable, Save } from "lucide-react";
import { useEffect, useMemo, useState, type FormEvent } from "react";
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
  snapshot: ConnectionsSnapshot;
  store: ConnectionsStore;
};

export function ConnectionEditor({ snapshot, store }: ConnectionEditorProps) {
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
    const nextErrors = validateConnectionForm(values, reservedServerIds);
    setErrors(nextErrors);
    if (Object.keys(nextErrors).length > 0) return;
    await store.save(connectionInputFromForm(values));
  }

  return (
    <form
      className="connections-editor"
      onSubmit={(event) => void submit(event)}
    >
      <header className="connections-editor__header">
        <span className="connections-icon connections-icon--large">
          <Cable aria-hidden="true" size={18} />
        </span>
        <span>
          <h2>Account & runtime / 账号与运行时</h2>
          <p>配置账号上下文并绑定一个独立的 MCP runtime。凭据只保存引用。</p>
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
          <h3 id="connection-provider-title">Provider 与 runtime</h3>
          <p>Provider 描述集成能力；runtime 是当前账号专用的 MCP 进程配置。</p>
        </header>
        <div className="connections-form-grid">
          <LabeledSelect
            disabled={snapshot.editorMode === "edit"}
            error={errors.definition}
            label={
              snapshot.editorMode === "edit"
                ? "Provider 定义（创建后不可更改）"
                : "Provider 定义"
            }
            options={snapshot.definitions.map((definition) => ({
              value: definition.id,
              label:
                definition.kind === "mcp"
                  ? definition.name
                  : `${definition.name}（后续支持）`,
              disabled: definition.kind !== "mcp" || !definition.enabled,
            }))}
            value={values.integrationDefinitionId}
            onChange={(integrationDefinitionId) =>
              setValues((current) => ({ ...current, integrationDefinitionId }))
            }
          />
          <LabeledSelect
            error={errors.server}
            label="MCP runtime"
            options={runtimeOptions(
              snapshot.mcpServers,
              reservedServerIds,
              editing,
            )}
            value={values.serverId}
            onChange={(serverId) =>
              setValues((current) => ({ ...current, serverId }))
            }
          />
          <TextField
            error={errors.name}
            label="Connection 名称"
            placeholder="例如：Salesforce · 张三"
            value={values.name}
            onChange={(event) =>
              setValues((current) => ({ ...current, name: event.target.value }))
            }
          />
          <TextField
            error={errors.environment}
            hint="例如 production、staging 或 cn-prod"
            label="Environment / 环境"
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
          每个 Connection 必须使用独立的 MCP runtime；legacy envKeys
          只是尚未验证的凭据来源，runtime ready 不代表账号认证健康。
        </p>
      </section>

      <section
        className="connections-form-section"
        aria-labelledby="connection-account-title"
      >
        <header>
          <h3 id="connection-account-title">账号与租户</h3>
          <p>这些字段帮助审批人确认外部操作实际使用的身份和数据范围。</p>
        </header>
        <div className="connections-form-grid">
          <LabeledSelect
            label="账号类型"
            options={(
              ["personal", "org_shared", "service_account"] as const
            ).map((ownerType) => ({
              value: ownerType,
              label: ownerTypeLabel(ownerType),
            }))}
            value={values.ownerType}
            onChange={(ownerType) =>
              setValues((current) => ({ ...current, ownerType }))
            }
          />
          <TextField
            label="账号显示名"
            placeholder="例如 张三 / sales-bot"
            value={values.accountDisplayName}
            onChange={(event) =>
              setValues((current) => ({
                ...current,
                accountDisplayName: event.target.value,
              }))
            }
          />
          <TextField
            label="外部账号 ID"
            value={values.externalAccountId}
            onChange={(event) =>
              setValues((current) => ({
                ...current,
                externalAccountId: event.target.value,
              }))
            }
          />
          <TextField
            label="Credential reference / 凭据引用"
            hint="只填写 Secret Store 的引用，不要粘贴 API Key 或 Token"
            placeholder="例如 vault://connections/sales"
            value={values.credentialRef}
            onChange={(event) =>
              setValues((current) => ({
                ...current,
                credentialRef: event.target.value,
              }))
            }
          />
          <TextField
            label="Tenant 名称"
            value={values.tenantName}
            onChange={(event) =>
              setValues((current) => ({
                ...current,
                tenantName: event.target.value,
              }))
            }
          />
          <TextField
            label="Tenant ID"
            value={values.tenantId}
            onChange={(event) =>
              setValues((current) => ({
                ...current,
                tenantId: event.target.value,
              }))
            }
          />
          <TextField
            label="Workspace 名称"
            value={values.workspaceName}
            onChange={(event) =>
              setValues((current) => ({
                ...current,
                workspaceName: event.target.value,
              }))
            }
          />
          <TextField
            label="Workspace ID"
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
          hint="使用英文逗号分隔；发布模板时仍需显式收窄操作授权"
          label="Granted scopes / 已授予范围"
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
            label="启用 Connection"
            onChange={(enabled) =>
              setValues((current) => ({ ...current, enabled }))
            }
          />
          <span>
            <strong>启用 Connection</strong>
            <small>停用后 Agent 和 Workflow 不得调用该账号。</small>
          </span>
        </label>
        <Button disabled={Boolean(saving)} type="submit" variant="primary">
          <Save aria-hidden="true" size={14} />
          {saving ? "保存中…" : "保存 Connection"}
        </Button>
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
          : [{ value: "" as T, label: "暂无可选项", disabled: true }]
      }
      value={value}
    />
  );
}

function runtimeOptions(
  servers: readonly McpServerView[],
  reservedServerIds: ReadonlySet<string>,
  editing: Connection | undefined,
) {
  return servers.map(({ server, status }) => {
    const reserved =
      reservedServerIds.has(server.serverId) &&
      server.serverId !== editing?.runtimeBinding.serverId;
    return {
      value: server.serverId,
      label: `${server.name} · ${status.status}${reserved ? " · 已绑定" : ""}`,
      disabled: reserved,
    };
  });
}
