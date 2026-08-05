import { useEffect, useMemo, useRef, useState } from "react";
import {
  Activity,
  AppWindow,
  ArrowLeft,
  CheckCircle2,
  CircleAlert,
  Play,
  RefreshCw,
  Save,
  ShieldCheck,
  Square,
} from "lucide-react";
import type { ApiClient } from "../api/client";
import { formatPathForDisplay } from "../pathDisplay";
import {
  activationForScope,
  parseJsonObject,
  permissionGrantForScope,
  pluginSettingFields,
} from "../pluginControl";
import type {
  AppViewDescriptor,
  AppViewSessionResponse,
  PluginControlScope,
  PluginControlScopeType,
  PluginDetail,
  PluginPermissionGrantRecord,
  PluginPermissionsResponse,
  PluginRuntimeHealthRecord,
  PluginSettingsResponse,
  PluginView,
} from "../types";
import { Badge, Button, NumberField, Select, Switch, TextField } from "./ui";

type PluginControlPanelProps = {
  client: ApiClient | null;
  pluginView: PluginView;
  threadId: string | null;
  workspaceRoot: string | null;
  onBack(): void;
};

export function PluginControlPanel({
  client,
  pluginView,
  threadId,
  workspaceRoot,
  onBack,
}: PluginControlPanelProps) {
  const initialScope = preferredScope(threadId, workspaceRoot);
  const [scopeType, setScopeType] = useState<PluginControlScopeType>(
    initialScope.scopeType,
  );
  const [detail, setDetail] = useState<PluginDetail | null>(null);
  const [settings, setSettings] = useState<PluginSettingsResponse | null>(null);
  const [permissions, setPermissions] =
    useState<PluginPermissionsResponse | null>(null);
  const [health, setHealth] = useState<PluginRuntimeHealthRecord[]>([]);
  const [appViews, setAppViews] = useState<AppViewDescriptor[]>([]);
  const [draftSettings, setDraftSettings] = useState<Record<string, unknown>>(
    {},
  );
  const [secretBindings, setSecretBindings] = useState<Record<string, string>>(
    {},
  );
  const [jsonDrafts, setJsonDrafts] = useState<Record<string, string>>({});
  const [constraintDrafts, setConstraintDrafts] = useState<
    Record<string, string>
  >({});
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const plugin = pluginView.plugin;
  const scope = useMemo(
    () => scopeForType(scopeType, threadId, workspaceRoot),
    [scopeType, threadId, workspaceRoot],
  );
  const context = useMemo(
    () => ({ workspaceRoot, threadId }),
    [threadId, workspaceRoot],
  );

  useEffect(() => {
    if (scopeType === "thread" && !threadId) {
      setScopeType(workspaceRoot ? "workspace" : "global");
    } else if (scopeType === "workspace" && !workspaceRoot) {
      setScopeType("global");
    }
  }, [scopeType, threadId, workspaceRoot]);

  useEffect(() => {
    let active = true;
    if (!client) {
      setLoading(false);
      setError("The local service is not connected.");
      return;
    }
    setLoading(true);
    setError(null);
    Promise.all([
      client.getPluginDetail(plugin.id, context),
      client.getPluginPermissions(plugin.id, context),
      client.getPluginContributions(plugin.id, context),
      client.getPluginHealth(plugin.id, context),
    ])
      .then(([nextDetail, nextPermissions, contributions, nextHealth]) => {
        if (!active) return;
        setDetail({ ...nextDetail, contributions, health: nextHealth });
        setPermissions(nextPermissions);
        setHealth(nextHealth);
        setConstraintDrafts(
          Object.fromEntries(
            nextPermissions.requests.map((request) => {
              const grant = permissionGrantForScope(
                nextPermissions.grants,
                scope,
                request.permission,
              );
              return [
                request.permission,
                JSON.stringify(grant?.constraint ?? {}, null, 2),
              ];
            }),
          ),
        );
      })
      .catch((caught) => active && setError(errorMessage(caught)))
      .finally(() => active && setLoading(false));
    return () => {
      active = false;
    };
  }, [client, context, plugin.id, scope]);

  useEffect(() => {
    let active = true;
    if (!client) return;
    client
      .getPluginSettings(plugin.id, scope)
      .then((response) => {
        if (!active) return;
        setSettings(response);
        const values = isRecord(response.settings.settings)
          ? response.settings.settings
          : {};
        setDraftSettings(values);
        setSecretBindings(
          Object.fromEntries(
            response.secretBindings.map((binding) => [
              binding.settingKey,
              binding.bindingId,
            ]),
          ),
        );
        const fields = pluginSettingFields(
          response.schema,
          detail?.manifest.secretSettingKeys ?? [],
        );
        setJsonDrafts(
          Object.fromEntries(
            fields
              .filter((field) => field.kind === "json")
              .map((field) => [
                field.key,
                JSON.stringify(
                  values[field.key] ?? field.defaultValue ?? {},
                  null,
                  2,
                ),
              ]),
          ),
        );
      })
      .catch((caught) => active && setError(errorMessage(caught)));
    return () => {
      active = false;
    };
  }, [client, detail?.manifest.secretSettingKeys, plugin.id, scope]);

  useEffect(() => {
    let active = true;
    if (!client || !threadId) {
      setAppViews([]);
      return;
    }
    client
      .getContributionHosts(threadId)
      .then((snapshot) => {
        if (active) {
          setAppViews(
            snapshot.apps.filter((app) => app.pluginId === plugin.id),
          );
        }
      })
      .catch((caught) => active && setError(errorMessage(caught)));
    return () => {
      active = false;
    };
  }, [client, detail?.effectiveEnabled, plugin.id, threadId]);

  const fields = useMemo(
    () =>
      pluginSettingFields(
        settings?.schema ?? detail?.manifest.configurationSchema,
        detail?.manifest.secretSettingKeys ?? [],
      ),
    [
      detail?.manifest.configurationSchema,
      detail?.manifest.secretSettingKeys,
      settings?.schema,
    ],
  );

  async function updateActivation(
    target: PluginControlScope,
    enabled: boolean,
  ) {
    if (!client) return;
    const key = `activation:${target.scopeType}`;
    setBusyKey(key);
    setError(null);
    try {
      await client.setPluginActivation(plugin.id, target, enabled);
      const nextDetail = await client.getPluginDetail(plugin.id, context);
      setDetail(nextDetail);
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setBusyKey(null);
    }
  }

  async function saveSettings() {
    if (!client) return;
    setBusyKey("settings");
    setError(null);
    try {
      const values = { ...draftSettings };
      for (const secretKey of detail?.manifest.secretSettingKeys ?? []) {
        delete values[secretKey];
      }
      for (const field of fields) {
        if (field.kind === "json") {
          values[field.key] = JSON.parse(jsonDrafts[field.key] ?? "null");
        }
      }
      const bindings = Object.fromEntries(
        (detail?.manifest.secretSettingKeys ?? []).map((key) => [
          key,
          secretBindings[key]?.trim() || null,
        ]),
      );
      const response = await client.updatePluginSettings(
        plugin.id,
        scope,
        values,
        bindings,
      );
      setSettings(response);
      setDraftSettings(
        isRecord(response.settings.settings) ? response.settings.settings : {},
      );
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setBusyKey(null);
    }
  }

  async function updatePermission(permission: string, granted: boolean) {
    if (!client || !permissions) return;
    const key = `permission:${permission}`;
    setBusyKey(key);
    setError(null);
    try {
      const constraint = parseJsonObject(constraintDrafts[permission] ?? "{}");
      const record = await client.setPluginPermission(plugin.id, {
        scope,
        permission,
        constraint,
        granted,
      });
      setPermissions({
        ...permissions,
        grants: replacePermissionGrant(permissions.grants, record),
      });
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setBusyKey(null);
    }
  }

  const scopeOptions: ReadonlyArray<{
    value: PluginControlScopeType;
    label: string;
    disabled: boolean;
  }> = [
    { value: "global", label: "Global", disabled: false },
    { value: "workspace", label: "Project", disabled: !workspaceRoot },
    { value: "thread", label: "Task", disabled: !threadId },
  ] as const;

  return (
    <div className="plugin-control-surface">
      <header className="plugin-control-header">
        <Button variant="quiet" size="compact" onClick={onBack}>
          <ArrowLeft size={14} />
          Plugins
        </Button>
        <div className="plugin-control-title">
          <strong>{plugin.displayName}</strong>
          <span>{plugin.description || plugin.name}</span>
        </div>
        <Badge variant={detail?.effectiveEnabled ? "success" : "neutral"}>
          {detail?.effectiveEnabled ? "Active" : "Inactive"}
        </Badge>
      </header>

      {error ? (
        <p className="plugin-control-error" role="alert">
          <CircleAlert size={14} />
          {error}
        </p>
      ) : null}
      {loading ? (
        <div className="plugin-control-loading">
          <RefreshCw className="spin" size={16} /> Loading plugin details
        </div>
      ) : null}

      <section
        className="plugin-control-section"
        aria-labelledby="plugin-activation-heading"
      >
        <div className="plugin-control-section-heading">
          <div>
            <h3 id="plugin-activation-heading">Activation</h3>
            <p>Each narrower scope can disable the plugin for that context.</p>
          </div>
        </div>
        <div className="plugin-activation-list">
          {scopeOptions.map((option) => {
            const target = scopeForType(option.value, threadId, workspaceRoot);
            const activation = detail
              ? activationForScope(detail.activations, target)
              : undefined;
            const fallback =
              option.value === "global" ? plugin.defaultEnabled : true;
            return (
              <div className="plugin-activation-row" key={option.value}>
                <div>
                  <strong>{option.label}</strong>
                  <span>
                    {option.disabled
                      ? option.value === "thread"
                        ? "Open a task to configure this scope."
                        : "Select a project to configure this scope."
                      : activation
                        ? `Explicitly ${activation.enabled ? "enabled" : "disabled"}`
                        : `Inherited default: ${fallback ? "enabled" : "disabled"}`}
                  </span>
                </div>
                <Switch
                  checked={activation?.enabled ?? fallback}
                  disabled={option.disabled || busyKey !== null}
                  label={`${option.label} plugin activation`}
                  onChange={(enabled) => void updateActivation(target, enabled)}
                />
              </div>
            );
          })}
        </div>
      </section>

      <section
        className="plugin-control-section"
        aria-labelledby="plugin-settings-heading"
      >
        <div className="plugin-control-section-heading">
          <div>
            <h3 id="plugin-settings-heading">Settings</h3>
            <p>{scopeDescription(scopeType, workspaceRoot, threadId)}</p>
          </div>
          <Select<PluginControlScopeType>
            label="Settings scope"
            value={scopeType}
            options={scopeOptions}
            onChange={setScopeType}
          />
        </div>
        {fields.length ? (
          <div className="plugin-settings-fields">
            {fields.map((field) => (
              <PluginSettingControl
                key={field.key}
                field={field}
                value={draftSettings[field.key] ?? field.defaultValue}
                secretBinding={secretBindings[field.key] ?? ""}
                jsonDraft={jsonDrafts[field.key] ?? "{}"}
                onChange={(value) =>
                  setDraftSettings((current) => ({
                    ...current,
                    [field.key]: value,
                  }))
                }
                onChangeSecret={(value) =>
                  setSecretBindings((current) => ({
                    ...current,
                    [field.key]: value,
                  }))
                }
                onChangeJson={(value) =>
                  setJsonDrafts((current) => ({
                    ...current,
                    [field.key]: value,
                  }))
                }
              />
            ))}
            <div className="plugin-control-actions">
              <Button
                variant="primary"
                size="compact"
                disabled={busyKey !== null}
                onClick={() => void saveSettings()}
              >
                <Save size={14} />
                {busyKey === "settings" ? "Saving" : "Save settings"}
              </Button>
            </div>
          </div>
        ) : (
          <p className="plugin-control-empty">
            This plugin has no configurable settings.
          </p>
        )}
      </section>

      <section
        className="plugin-control-section"
        aria-labelledby="plugin-permissions-heading"
      >
        <div className="plugin-control-section-heading">
          <div>
            <h3 id="plugin-permissions-heading">Permissions</h3>
            <p>Grants below apply only to the selected {scopeType} scope.</p>
          </div>
          <ShieldCheck size={16} />
        </div>
        {permissions?.requests.length ? (
          <div className="plugin-permission-list">
            {permissions.requests.map((request) => {
              const grant = permissionGrantForScope(
                permissions.grants,
                scope,
                request.permission,
              );
              const granted = grant?.status === "granted";
              return (
                <div className="plugin-permission-row" key={request.permission}>
                  <div className="plugin-permission-heading">
                    <div>
                      <strong>{request.permission}</strong>
                      <span>
                        {request.category}: {request.value}
                      </span>
                    </div>
                    <Badge
                      variant={
                        granted ? "success" : grant ? "danger" : "neutral"
                      }
                    >
                      {granted ? "Granted" : grant ? "Revoked" : "Not set"}
                    </Badge>
                  </div>
                  <label className="plugin-json-field">
                    <span>Constraint</span>
                    <textarea
                      value={constraintDrafts[request.permission] ?? "{}"}
                      spellCheck={false}
                      onChange={(event) =>
                        setConstraintDrafts((current) => ({
                          ...current,
                          [request.permission]: event.target.value,
                        }))
                      }
                    />
                  </label>
                  <div className="plugin-control-actions">
                    <Button
                      size="compact"
                      variant={granted ? "danger" : "secondary"}
                      disabled={busyKey !== null}
                      onClick={() =>
                        void updatePermission(request.permission, !granted)
                      }
                    >
                      {granted ? "Revoke" : "Grant"}
                    </Button>
                  </div>
                </div>
              );
            })}
          </div>
        ) : (
          <p className="plugin-control-empty">
            This plugin requests no permissions.
          </p>
        )}
      </section>

      <section
        className="plugin-control-section"
        aria-labelledby="plugin-contributions-heading"
      >
        <div className="plugin-control-section-heading">
          <div>
            <h3 id="plugin-contributions-heading">Contributions</h3>
            <p>Capabilities registered by this plugin.</p>
          </div>
          <Activity size={16} />
        </div>
        {detail?.contributions.length ? (
          <div className="plugin-contribution-list">
            {detail.contributions.map((contribution) => {
              const runtime = health.find(
                (record) =>
                  record.contributionId === contribution.contributionId,
              );
              return (
                <div
                  className="plugin-contribution-row"
                  key={contribution.contributionId}
                >
                  <div>
                    <strong>{contribution.localId}</strong>
                    <code>{contribution.contributionId}</code>
                  </div>
                  <div className="plugin-contribution-status">
                    <Badge>{contribution.kind}</Badge>
                    <HealthBadge health={runtime} />
                  </div>
                  {runtime?.lastError ? (
                    <p className="plugin-health-error">{runtime.lastError}</p>
                  ) : null}
                </div>
              );
            })}
          </div>
        ) : (
          <p className="plugin-control-empty">No registered contributions.</p>
        )}
      </section>

      {plugin.hasApps || appViews.length ? (
        <section
          className="plugin-control-section"
          aria-labelledby="plugin-app-views-heading"
        >
          <div className="plugin-control-section-heading">
            <div>
              <h3 id="plugin-app-views-heading">App views</h3>
            </div>
            <AppWindow size={16} />
          </div>
          {!threadId ? (
            <p className="plugin-control-empty">
              Open a task to start this plugin&apos;s app views.
            </p>
          ) : client && appViews.length ? (
            <div className="plugin-app-list">
              {appViews.map((app) => (
                <PluginAppView
                  key={app.contributionId}
                  client={client}
                  threadId={threadId}
                  descriptor={app}
                />
              ))}
            </div>
          ) : (
            <p className="plugin-control-empty">
              No active app views are available for this task.
            </p>
          )}
        </section>
      ) : null}
    </div>
  );
}

function PluginAppView({
  client,
  threadId,
  descriptor,
}: {
  client: ApiClient;
  threadId: string;
  descriptor: AppViewDescriptor;
}) {
  const frameRef = useRef<HTMLIFrameElement>(null);
  const mountedRef = useRef(true);
  const sessionRef = useRef<AppViewSessionResponse | null>(null);
  const contentUrlRef = useRef<string | null>(null);
  const [session, setSession] = useState<AppViewSessionResponse | null>(null);
  const [contentUrl, setContentUrl] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      const currentSession = sessionRef.current;
      const currentUrl = contentUrlRef.current;
      if (currentUrl) URL.revokeObjectURL(currentUrl);
      if (currentSession) {
        void client
          .stopPluginAppSession(threadId, currentSession.sessionId)
          .catch(() => undefined);
      }
    };
  }, [client, threadId]);

  useEffect(() => {
    if (!session) return;
    const receiveMessage = (event: MessageEvent<unknown>) => {
      const target = frameRef.current?.contentWindow;
      if (!target || event.source !== target || !isRecord(event.data)) return;
      if (event.data.api !== "appView.postMessage.v1") return;
      const channel = event.data.channel;
      if (
        typeof channel !== "string" ||
        !descriptor.allowedChannels.includes(channel)
      ) {
        target.postMessage(
          {
            api: "appView.postMessage.v1.result",
            ok: false,
            error: "Channel is not allowed by the app manifest.",
          },
          "*",
        );
        return;
      }
      void client
        .postPluginAppMessage(
          threadId,
          session.sessionId,
          channel,
          event.data.payload,
        )
        .then((message) =>
          target.postMessage(
            { api: "appView.postMessage.v1.result", ok: true, message },
            "*",
          ),
        )
        .catch((caught) =>
          target.postMessage(
            {
              api: "appView.postMessage.v1.result",
              ok: false,
              error: errorMessage(caught),
            },
            "*",
          ),
        );
    };
    window.addEventListener("message", receiveMessage);
    return () => window.removeEventListener("message", receiveMessage);
  }, [client, descriptor.allowedChannels, session, threadId]);

  async function start() {
    setBusy(true);
    setError(null);
    let nextSession: AppViewSessionResponse | null = null;
    try {
      nextSession = await client.startPluginAppSession(
        threadId,
        descriptor.contributionId,
      );
      sessionRef.current = nextSession;
      if (!mountedRef.current) {
        sessionRef.current = null;
        void client
          .stopPluginAppSession(threadId, nextSession.sessionId)
          .catch(() => undefined);
        return;
      }
      const source = await client.getPluginAppContent(
        threadId,
        nextSession.sessionId,
      );
      const document = buildSandboxedAppDocument(source);
      const nextUrl = URL.createObjectURL(
        new Blob([document], { type: "text/html" }),
      );
      if (!mountedRef.current) {
        URL.revokeObjectURL(nextUrl);
        return;
      }
      contentUrlRef.current = nextUrl;
      setSession(nextSession);
      setContentUrl(nextUrl);
    } catch (caught) {
      if (nextSession) {
        sessionRef.current = null;
        void client
          .stopPluginAppSession(threadId, nextSession.sessionId)
          .catch(() => undefined);
      }
      if (mountedRef.current) setError(errorMessage(caught));
    } finally {
      if (mountedRef.current) setBusy(false);
    }
  }

  async function stop() {
    const currentSession = sessionRef.current;
    const currentUrl = contentUrlRef.current;
    setBusy(true);
    setError(null);
    try {
      if (currentSession) {
        await client.stopPluginAppSession(threadId, currentSession.sessionId);
      }
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      if (currentUrl) URL.revokeObjectURL(currentUrl);
      sessionRef.current = null;
      contentUrlRef.current = null;
      if (mountedRef.current) {
        setSession(null);
        setContentUrl(null);
        setBusy(false);
      }
    }
  }

  return (
    <div className="plugin-app-view">
      <div className="plugin-app-heading">
        <div>
          <strong>{descriptor.title}</strong>
          <code>{descriptor.contributionId}</code>
        </div>
        <div className="plugin-app-actions">
          <Badge variant={session ? "success" : "neutral"}>
            {session ? "Running" : "Stopped"}
          </Badge>
          {session ? (
            <Button
              size="compact"
              variant="secondary"
              disabled={busy}
              onClick={() => void stop()}
            >
              <Square size={14} />
              Stop
            </Button>
          ) : (
            <Button
              size="compact"
              variant="secondary"
              disabled={busy}
              onClick={() => void start()}
            >
              <Play size={14} />
              {busy ? "Starting" : "Start"}
            </Button>
          )}
        </div>
      </div>
      {error ? (
        <p className="plugin-app-error" role="alert">
          <CircleAlert size={14} />
          {error}
        </p>
      ) : null}
      {contentUrl ? (
        <iframe
          ref={frameRef}
          className="plugin-app-frame"
          src={contentUrl}
          title={descriptor.title}
          sandbox="allow-scripts"
          referrerPolicy="no-referrer"
        />
      ) : (
        <div className="plugin-app-placeholder">
          <AppWindow size={20} />
          <span>App view is stopped.</span>
        </div>
      )}
    </div>
  );
}

function buildSandboxedAppDocument(source: string): string {
  const document = new DOMParser().parseFromString(source, "text/html");
  document.querySelectorAll("base").forEach((element) => element.remove());
  document
    .querySelectorAll('meta[http-equiv="Content-Security-Policy" i]')
    .forEach((element) => element.remove());
  const policy = document.createElement("meta");
  policy.httpEquiv = "Content-Security-Policy";
  policy.content =
    "default-src 'none'; img-src data: blob:; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'";
  document.head.prepend(policy);
  return `<!doctype html>\n${document.documentElement.outerHTML}`;
}

function PluginSettingControl({
  field,
  value,
  secretBinding,
  jsonDraft,
  onChange,
  onChangeSecret,
  onChangeJson,
}: {
  field: ReturnType<typeof pluginSettingFields>[number];
  value: unknown;
  secretBinding: string;
  jsonDraft: string;
  onChange(value: unknown): void;
  onChangeSecret(value: string): void;
  onChangeJson(value: string): void;
}) {
  const label = `${field.label}${field.required ? " (required)" : ""}`;
  if (field.kind === "boolean") {
    return (
      <div className="plugin-setting-switch">
        <div>
          <strong>{label}</strong>
          {field.description ? <span>{field.description}</span> : null}
        </div>
        <Switch
          checked={typeof value === "boolean" ? value : false}
          label={label}
          onChange={onChange}
        />
      </div>
    );
  }
  if (field.kind === "enum") {
    const selected =
      typeof value === "string" && field.enumValues.includes(value)
        ? value
        : (field.enumValues[0] ?? "");
    return (
      <label className="plugin-labeled-control">
        <span>{label}</span>
        <Select
          label={label}
          value={selected}
          options={field.enumValues.map((option) => ({
            value: option,
            label: option,
          }))}
          onChange={onChange}
        />
        {field.description ? <small>{field.description}</small> : null}
      </label>
    );
  }
  if (field.kind === "number" || field.kind === "integer") {
    return (
      <label className="plugin-labeled-control">
        <span>{label}</span>
        <NumberField
          label={label}
          value={typeof value === "number" ? value : 0}
          min={field.minimum}
          max={field.maximum}
          step={field.kind === "integer" ? 1 : "any"}
          onChange={onChange}
        />
        {field.description ? <small>{field.description}</small> : null}
      </label>
    );
  }
  if (field.kind === "secret") {
    return (
      <TextField
        label={label}
        value={secretBinding}
        placeholder="Opaque binding ID"
        hint={field.description ?? "Reference a stored secret by binding ID."}
        onChange={(event) => onChangeSecret(event.target.value)}
      />
    );
  }
  if (field.kind === "json") {
    return (
      <label className="plugin-json-field">
        <span>{label}</span>
        <textarea
          value={jsonDraft}
          spellCheck={false}
          onChange={(event) => onChangeJson(event.target.value)}
        />
        {field.description ? <small>{field.description}</small> : null}
      </label>
    );
  }
  return (
    <TextField
      label={label}
      value={typeof value === "string" ? value : ""}
      hint={field.description}
      onChange={(event) => onChange(event.target.value)}
    />
  );
}

function HealthBadge({ health }: { health?: PluginRuntimeHealthRecord }) {
  if (!health) return <Badge>Not started</Badge>;
  const variant =
    health.status === "ready"
      ? "success"
      : health.status === "degraded"
        ? "warning"
        : health.status === "error"
          ? "danger"
          : "neutral";
  return (
    <Badge variant={variant}>
      {health.status === "ready" ? <CheckCircle2 size={12} /> : null}
      {health.status}
    </Badge>
  );
}

function preferredScope(
  threadId: string | null,
  workspaceRoot: string | null,
): PluginControlScope {
  if (threadId) return { scopeType: "thread", scopeId: threadId };
  if (workspaceRoot) return { scopeType: "workspace", scopeId: workspaceRoot };
  return { scopeType: "global" };
}

function scopeForType(
  scopeType: PluginControlScopeType,
  threadId: string | null,
  workspaceRoot: string | null,
): PluginControlScope {
  if (scopeType === "thread") {
    return { scopeType, scopeId: threadId ?? undefined };
  }
  if (scopeType === "workspace") {
    return { scopeType, scopeId: workspaceRoot ?? undefined };
  }
  return { scopeType };
}

function scopeDescription(
  scopeType: PluginControlScopeType,
  workspaceRoot: string | null,
  threadId: string | null,
): string {
  if (scopeType === "thread") return `Task: ${threadId ?? "unavailable"}`;
  if (scopeType === "workspace")
    return `Project: ${
      workspaceRoot ? formatPathForDisplay(workspaceRoot) : "unavailable"
    }`;
  return "Applies across OpenTopia unless overridden by a narrower scope.";
}

function replacePermissionGrant(
  grants: PluginPermissionGrantRecord[],
  record: PluginPermissionGrantRecord,
): PluginPermissionGrantRecord[] {
  return [
    ...grants.filter(
      (grant) =>
        !(
          grant.permission === record.permission &&
          grant.scope.scopeType === record.scope.scopeType &&
          grant.scope.scopeId === record.scope.scopeId
        ),
    ),
    record,
  ];
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
