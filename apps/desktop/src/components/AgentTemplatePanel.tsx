import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Bot,
  GitCompareArrows,
  Plus,
  RefreshCw,
  ShieldAlert,
  Sparkles,
  Trash2,
  UserRoundCog,
} from "lucide-react";
import type { ApiClient } from "../api/client";
import { useApplicationLanguage } from "../ApplicationLanguageProvider";
import type {
  AgentInstance,
  AgentTemplateVersionView,
  AppSettings,
  ExecutionResourceGrant,
} from "../types";
import type { AgentTemplateConnectionAccessView } from "../api/generated/desktop-http-v1.generated";
import {
  agentKnowledgeBindingSummary,
  agentToolsWithKnowledgeAccess,
} from "../agentKnowledgeBinding";
import { Badge, Button, Panel, SelectField, TextField } from "./ui";
import {
  useEnterpriseSubpageHeader,
  type EnterprisePageHeaderChange,
} from "./enterprise/pageHeader";
import { useFlowAgentSelection } from "./enterprise/flowAgentSelection";
import {
  AgentTemplateConnectionAccessSummary,
  AgentTemplateConnectionGrantsField,
} from "./agentTemplateConnectionGrants";
import { AgentTemplateKnowledgeBindingField } from "./AgentTemplateKnowledgeBindingField";
import { AgentConfigInspector } from "./AgentConfigInspector";
import { generateAgentDraftWithModel } from "./agentAuthoring/generateAgentDraftWithModel";
import {
  agentDraftFromTemplate,
  blankAgentDraft,
  parseAgentDraftJson,
  parseAgentDraftList,
  parseAgentModelBindings,
  setAgentDraftValue as setFormValue,
  type AgentDraftForm,
} from "./agentAuthoring/agentDraftForm";
import {
  AgentTextAreaField as TextAreaField,
  agentCapabilitySummary as capabilitySummary,
  agentChangeKindLabel as changeKindLabel,
  agentInstanceStatusLabel as instanceStatusLabel,
  agentRiskBadge as riskBadge,
  agentRiskLabel as riskLabel,
  shortAgentId as shortId,
} from "./agentAuthoring/agentPresentation";
import "../styles/agent-template-panel.css";

type AgentTemplatePanelProps = {
  client: ApiClient | null;
  threadId: string | null;
  workspaceRoot: string | null;
  settings: AppSettings | null;
  onPageHeaderChange?: EnterprisePageHeaderChange;
  showTemplateCollection?: boolean;
  variant?: "default" | "rail";
};

export function AgentTemplatePanel({
  client,
  threadId,
  workspaceRoot,
  settings,
  onPageHeaderChange,
  showTemplateCollection = true,
  variant = "default",
}: AgentTemplatePanelProps) {
  const { language, t } = useApplicationLanguage();
  const selection = useFlowAgentSelection();
  const [templates, setTemplates] = useState<AgentTemplateVersionView[]>([]);
  const [instances, setInstances] = useState<AgentInstance[]>([]);
  const [boundInstance, setBoundInstance] = useState<AgentInstance | null>(
    null,
  );
  const [localSelectedKey, setLocalSelectedKey] = useState<string | null>(null);
  const [form, setForm] = useState<AgentDraftForm>(() =>
    blankAgentDraft(workspaceRoot, settings, language),
  );
  const [requirement, setRequirement] = useState("");
  const [editing, setEditing] = useState(Boolean(selection?.creatingAgent));
  const [initialState, setInitialState] = useState("{}");
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [connectionAccess, setConnectionAccess] =
    useState<AgentTemplateConnectionAccessView | null>(null);
  const [connectionAccessLoading, setConnectionAccessLoading] = useState(false);
  const [connectionAccessError, setConnectionAccessError] = useState<
    string | null
  >(null);
  const [connectionAccessRefresh, setConnectionAccessRefresh] = useState(0);
  const createAgentRequest = selection?.createAgentRequest ?? 0;
  const viewAgentRequest = selection?.viewAgentRequest ?? 0;
  const handledCreateAgentRequest = useRef(createAgentRequest);
  const handledViewAgentRequest = useRef(viewAgentRequest);
  const notifyAgentDataChanged = selection?.notifyAgentDataChanged;

  const cancelCreateAgent = selection?.cancelCreateAgent;
  const closeEditor = useCallback(() => {
    setEditing(false);
    cancelCreateAgent?.();
  }, [cancelCreateAgent]);
  useEnterpriseSubpageHeader(onPageHeaderChange, editing, {
    title: t("flow.agentEditor.subpageTitle"),
    backLabel: t("flow.agentEditor.back"),
    onBack: closeEditor,
  });

  const sharedSetSelectedKey = selection?.setSelectedTemplateKey;
  const selectedKey = selection?.selectedTemplateKey ?? localSelectedKey;
  const selectedKeyRef = useRef(selectedKey);
  selectedKeyRef.current = selectedKey;
  const setSelectedKey = useCallback(
    (key: string | null) => {
      if (sharedSetSelectedKey) sharedSetSelectedKey(key);
      else setLocalSelectedKey(key);
    },
    [sharedSetSelectedKey],
  );
  const selected = useMemo(
    () => templates.find((view) => templateKey(view) === selectedKey) ?? null,
    [selectedKey, templates],
  );

  useEffect(() => {
    if (createAgentRequest <= handledCreateAgentRequest.current) return;
    handledCreateAgentRequest.current = createAgentRequest;
    startNewVersion();
  }, [createAgentRequest]);

  useEffect(() => {
    if (viewAgentRequest <= handledViewAgentRequest.current) return;
    handledViewAgentRequest.current = viewAgentRequest;
    closeEditor();
  }, [closeEditor, viewAgentRequest]);

  const refresh = useCallback(async () => {
    if (!client) {
      setTemplates([]);
      setInstances([]);
      setBoundInstance(null);
      return;
    }
    setError(null);
    try {
      const [nextTemplates, nextInstances, nextBound] = await Promise.all([
        client.listAgentTemplates(),
        threadId
          ? client.listThreadAgentInstances(threadId)
          : Promise.resolve([]),
        threadId
          ? client.getBoundThreadAgentInstance(threadId)
          : Promise.resolve(null),
      ]);
      setTemplates(nextTemplates);
      setInstances(nextInstances);
      setBoundInstance(nextBound);
      setConnectionAccessRefresh((current) => current + 1);
      const currentSelectedKey = selectedKeyRef.current;
      const nextSelectedKey = (() => {
        if (
          currentSelectedKey &&
          nextTemplates.some((view) => templateKey(view) === currentSelectedKey)
        ) {
          return currentSelectedKey;
        }
        return nextTemplates[0] ? templateKey(nextTemplates[0]) : null;
      })();
      setSelectedKey(nextSelectedKey);
    } catch (refreshError) {
      setError(readableError(refreshError));
    }
  }, [client, setSelectedKey, threadId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const refreshAfterMutation = useCallback(async () => {
    await refresh();
    notifyAgentDataChanged?.();
  }, [notifyAgentDataChanged, refresh]);

  useEffect(() => {
    if (!client || !selected) {
      setConnectionAccess(null);
      setConnectionAccessError(null);
      setConnectionAccessLoading(false);
      return;
    }
    const controller = new AbortController();
    setConnectionAccess(null);
    setConnectionAccessError(null);
    setConnectionAccessLoading(true);
    void client
      .getAgentTemplateConnectionAccess(
        selected.template.templateId,
        selected.template.version,
        controller.signal,
      )
      .then((access) => {
        if (!controller.signal.aborted) setConnectionAccess(access);
      })
      .catch((accessError: unknown) => {
        if (!controller.signal.aborted) {
          setConnectionAccessError(readableError(accessError));
        }
      })
      .finally(() => {
        if (!controller.signal.aborted) setConnectionAccessLoading(false);
      });
    return () => controller.abort();
  }, [client, connectionAccessRefresh, selected]);

  useEffect(() => {
    if (!editing) setForm(blankAgentDraft(workspaceRoot, settings, language));
  }, [editing, language, settings, workspaceRoot]);

  async function createVersion() {
    if (!client || busy) return;
    setBusy("create");
    setError(null);
    setNotice(null);
    try {
      const resourceGrants = parseAgentDraftJson<ExecutionResourceGrant[]>(
        form.resourceGrants,
        t("flow.agentEditor.resourceBindings"),
        language,
      );
      if (!Array.isArray(resourceGrants)) {
        throw new Error(t("flow.agentEditor.resourceBindingsArray"));
      }
      const stateSchema = parseAgentDraftJson<unknown>(
        form.stateSchema,
        t("flow.agentEditor.stateSchema"),
        language,
      );
      const outputSchema = parseAgentDraftJson<unknown>(
        form.outputSchema,
        t("flow.agentEditor.outputSchema"),
        language,
      );
      const knowledgeNamespaces = parseAgentDraftList(form.knowledgeNamespaces);
      if (
        form.knowledgeProvider === "sag" &&
        knowledgeNamespaces.length === 0
      ) {
        throw new Error(t("flow.agentEditor.sagNamespaceRequired"));
      }
      const tools = parseAgentDraftList(form.tools);
      const created = await client.createAgentTemplateVersion({
        templateId: form.templateId.trim(),
        name: form.name.trim(),
        owner: form.owner.trim(),
        spec: {
          description: form.description.trim(),
          instructions: form.instructions.trim(),
          capabilities: {
            allowAllTools: false,
            tools,
            allowAllSkills: false,
            skills: parseAgentDraftList(form.skills),
            allowAllPlugins: false,
            plugins: parseAgentDraftList(form.plugins),
            allowAllMcpServers: form.legacyAllowAllMcpServers,
            mcpServers: parseAgentDraftList(form.mcpServers),
            allowAllWorkspaceRoots: false,
            workspaceRoots: parseAgentDraftList(form.workspaceRoots),
          },
          connectionBindings: form.connectionBindings,
          knowledgeBinding: form.knowledgeProvider
            ? {
                provider: form.knowledgeProvider,
                namespaces:
                  form.knowledgeProvider === "sag" ? knowledgeNamespaces : [],
              }
            : undefined,
          resourceGrants,
          modelPolicy: {
            allowAllModels: false,
            allowedModels: parseAgentModelBindings(form.models, language),
          },
          stateSchema,
          outputSchema,
          allowAllDelegates: false,
          delegateTemplateIds: parseAgentDraftList(form.delegates),
          budget: {
            maxTurns: 20,
            maxToolCalls: 40,
            maxDurationSeconds: 900,
          },
          riskClass: form.riskClass,
        },
      });
      setEditing(false);
      setNotice(
        `${t("flow.agentEditor.created")} ${created.template.templateId}@${created.template.version}`,
      );
      await refreshAfterMutation();
      setSelectedKey(templateKey(created));
    } catch (createError) {
      setError(readableError(createError));
    } finally {
      setBusy(null);
    }
  }

  async function generateWithModel() {
    if (!client || !threadId || busy || !requirement.trim()) return;
    setBusy("generate");
    setError(null);
    setNotice(null);
    try {
      const generated = await generateAgentDraftWithModel({
        client,
        threadId,
        requirement,
        existingTemplates: templates,
        settings,
      });
      setForm(
        agentDraftFromTemplate(generated, workspaceRoot, settings, language),
      );
      setSelectedKey(templateKey(generated));
      setNotice(
        `${t("flow.agentEditor.generatedPrefix")} ${generated.template.name}${t("flow.agentEditor.generatedSuffix")}`,
      );
      await refreshAfterMutation();
      setSelectedKey(templateKey(generated));
    } catch (generationError) {
      setError(readableError(generationError));
    } finally {
      setBusy(null);
    }
  }

  async function publishSelected() {
    if (!client || !selected || busy) return;
    setBusy("publish");
    setError(null);
    setNotice(null);
    try {
      await client.publishAgentTemplateVersion(
        selected.template.templateId,
        selected.template.version,
        {
          approvedBy: selected.template.owner,
          approveCapabilityExpansion: selected.diff.widensCapabilities,
        },
      );
      setNotice(t("flow.agentEditor.publishedNotice"));
      await refreshAfterMutation();
    } catch (publishError) {
      setError(readableError(publishError));
    } finally {
      setBusy(null);
    }
  }

  async function deleteSelected() {
    if (!client || !selected || busy) return;
    setBusy("delete");
    setError(null);
    setNotice(null);
    try {
      await client.deleteAgentTemplateVersion(
        selected.template.templateId,
        selected.template.version,
      );
      setNotice(t("flow.agentEditor.deletedNotice"));
      await refreshAfterMutation();
    } catch (deleteError) {
      setError(readableError(deleteError));
    } finally {
      setBusy(null);
    }
  }

  async function archiveSelected() {
    if (!client || !selected || busy) return;
    setBusy("archive");
    setError(null);
    setNotice(null);
    try {
      await client.archiveAgentTemplate(selected.template.templateId);
      setNotice(t("flow.agentEditor.archivedNotice"));
      await refreshAfterMutation();
    } catch (archiveError) {
      setError(readableError(archiveError));
    } finally {
      setBusy(null);
    }
  }

  async function instantiateSelected() {
    if (!client || !selected || !threadId || busy) return;
    setBusy("instantiate");
    setError(null);
    setNotice(null);
    try {
      const state = parseAgentDraftJson<unknown>(
        initialState,
        t("flow.agentEditor.instanceState"),
        language,
      );
      const response = await client.createAgentInstance({
        templateId: selected.template.templateId,
        templateVersion: selected.template.version,
        threadId,
        initialState: state,
        bindToThread: true,
      });
      setNotice(
        `${t("flow.agentEditor.createdBound")} ${shortId(response.instance.id)}`,
      );
      await refreshAfterMutation();
    } catch (instantiateError) {
      setError(readableError(instantiateError));
    } finally {
      setBusy(null);
    }
  }

  async function bindInstance(instanceId: string) {
    if (!client || !threadId || busy) return;
    setBusy(`bind:${instanceId}`);
    setError(null);
    try {
      await client.bindThreadAgentInstance(threadId, instanceId);
      setNotice(`${t("flow.agentEditor.switched")} ${shortId(instanceId)}`);
      await refreshAfterMutation();
    } catch (bindError) {
      setError(readableError(bindError));
    } finally {
      setBusy(null);
    }
  }

  async function revokeInstance(instanceId: string) {
    if (!client || busy) return;
    setBusy(`revoke:${instanceId}`);
    setError(null);
    try {
      await client.updateAgentInstance(instanceId, { status: "revoked" });
      setNotice(`${t("flow.agentEditor.revoked")} ${shortId(instanceId)}`);
      await refreshAfterMutation();
    } catch (revokeError) {
      setError(readableError(revokeError));
    } finally {
      setBusy(null);
    }
  }

  function startNewVersion(source?: AgentTemplateVersionView) {
    setError(null);
    setNotice(null);
    setForm(
      source
        ? agentDraftFromTemplate(source, workspaceRoot, settings, language)
        : blankAgentDraft(workspaceRoot, settings, language),
    );
    setRequirement(source?.template.spec.description ?? "");
    setEditing(true);
  }

  return (
    <div
      className={`agent-template-panel agent-template-panel--${variant}`}
      aria-label={t("flow.agentEditor.aria")}
    >
      {!editing && showTemplateCollection ? (
        <Panel
          title={t("flow.agentEditor.configTitle")}
          actions={
            <div className="agent-template-panel__header-actions">
              <Badge variant="warning">{t("flow.agents.draft")}</Badge>
              <Button
                size="compact"
                variant="quiet"
                aria-label={t("flow.agentEditor.refreshAria")}
                disabled={!client || Boolean(busy)}
                onClick={() => void refresh()}
              >
                <RefreshCw size={14} aria-hidden="true" />
              </Button>
              <Button
                size="compact"
                variant="primary"
                disabled={!client || Boolean(busy)}
                onClick={() => startNewVersion()}
              >
                <Plus size={14} aria-hidden="true" />
                {t("flow.agentEditor.new")}
              </Button>
            </div>
          }
        >
          {templates.length ? (
            <div className="agent-template-panel__template-list" role="list">
              {templates.map((view) => {
                const active = templateKey(view) === selectedKey;
                return (
                  <button
                    className={`agent-template-panel__template ${active ? "is-active" : ""}`}
                    key={templateKey(view)}
                    type="button"
                    aria-pressed={active}
                    onClick={() => setSelectedKey(templateKey(view))}
                  >
                    <Bot size={16} aria-hidden="true" />
                    <span>
                      <strong>{view.template.name}</strong>
                      <small>
                        {view.template.templateId}@{view.template.version}
                      </small>
                    </span>
                    <Badge
                      variant={
                        view.template.status === "published"
                          ? "success"
                          : "warning"
                      }
                    >
                      {view.template.status === "published"
                        ? t("flow.agents.published")
                        : t("flow.agents.draft")}
                    </Badge>
                  </button>
                );
              })}
            </div>
          ) : (
            <p className="agent-template-panel__empty">
              {t("flow.agentEditor.none")}
            </p>
          )}
        </Panel>
      ) : null}

      {editing ? (
        <Panel
          title={t("flow.agentEditor.createTitle")}
          actions={
            <div className="agent-template-panel__header-actions">
              <Button
                disabled={Boolean(busy)}
                onClick={closeEditor}
                size="compact"
                variant="quiet"
              >
                {t("flow.agentEditor.cancel")}
              </Button>
              <Button
                disabled={!client || busy === "create"}
                onClick={() => void createVersion()}
                size="compact"
                variant="primary"
              >
                {busy === "create"
                  ? t("flow.agentEditor.saving")
                  : t("flow.agentEditor.saveVersion")}
              </Button>
            </div>
          }
        >
          <div className="agent-studio">
            <main className="agent-studio__main">
              <section className="agent-studio__composer">
                <span>
                  <strong>{t("flow.agentEditor.describeTitle")}</strong>
                  <small>{t("flow.agentEditor.describeDetail")}</small>
                </span>
                <textarea
                  onChange={(event) => setRequirement(event.target.value)}
                  placeholder={t("flow.agentEditor.describePlaceholder")}
                  value={requirement}
                />
                <div className="agent-studio__composer-actions">
                  <small>{t("flow.agentEditor.generateHint")}</small>
                  <Button
                    disabled={!threadId || !requirement.trim() || Boolean(busy)}
                    onClick={() => void generateWithModel()}
                    variant="primary"
                  >
                    <Sparkles aria-hidden="true" size={14} />
                    {busy === "generate"
                      ? t("flow.agentEditor.generating")
                      : t("flow.agentEditor.generate")}
                  </Button>
                </div>
              </section>
              <div className="agent-template-panel__form">
                <TextField
                  label={t("flow.agentEditor.agentId")}
                  value={form.templateId}
                  onChange={(event) =>
                    setFormValue(setForm, "templateId", event.target.value)
                  }
                />
                <TextField
                  label={t("flow.agentEditor.name")}
                  value={form.name}
                  onChange={(event) =>
                    setFormValue(setForm, "name", event.target.value)
                  }
                />
                <TextField
                  label={t("flow.agentEditor.owner")}
                  value={form.owner}
                  onChange={(event) =>
                    setFormValue(setForm, "owner", event.target.value)
                  }
                />
                <TextField
                  label={t("flow.agentEditor.description")}
                  value={form.description}
                  onChange={(event) =>
                    setFormValue(setForm, "description", event.target.value)
                  }
                />
                <TextAreaField
                  label={t("flow.agentEditor.instructions")}
                  value={form.instructions}
                  onChange={(value) =>
                    setFormValue(setForm, "instructions", value)
                  }
                />
                <AgentTemplateConnectionGrantsField
                  client={client}
                  disabled={Boolean(busy)}
                  legacyAllowAllMcpServers={form.legacyAllowAllMcpServers}
                  legacyMcpServerIds={parseAgentDraftList(form.mcpServers)}
                  value={form.connectionBindings}
                  onChange={(connectionBindings) =>
                    setFormValue(
                      setForm,
                      "connectionBindings",
                      connectionBindings,
                    )
                  }
                  onClearLegacyMcpServers={() =>
                    setForm((current) => ({
                      ...current,
                      legacyAllowAllMcpServers: false,
                      mcpServers: "",
                    }))
                  }
                />
                <AgentTemplateKnowledgeBindingField
                  disabled={Boolean(busy)}
                  provider={form.knowledgeProvider}
                  namespaces={form.knowledgeNamespaces}
                  onProviderChange={(knowledgeProvider) =>
                    setFormValue(
                      setForm,
                      "knowledgeProvider",
                      knowledgeProvider,
                    )
                  }
                  onNamespacesChange={(knowledgeNamespaces) =>
                    setFormValue(
                      setForm,
                      "knowledgeNamespaces",
                      knowledgeNamespaces,
                    )
                  }
                />
                <SelectField
                  fieldClassName="agent-template-panel__field"
                  label={t("flow.agentEditor.risk")}
                  value={form.riskClass}
                  onChange={(value) =>
                    setFormValue(
                      setForm,
                      "riskClass",
                      value as AgentDraftForm["riskClass"],
                    )
                  }
                  options={[
                    { value: "low", label: t("flow.agentEditor.riskLow") },
                    {
                      value: "medium",
                      label: t("flow.agentEditor.riskMedium"),
                    },
                    { value: "high", label: t("flow.agentEditor.riskHigh") },
                    {
                      value: "critical",
                      label: t("flow.agentEditor.riskCritical"),
                    },
                  ]}
                />
                <details className="agent-template-panel__advanced">
                  <summary>{t("flow.agentEditor.advanced")}</summary>
                  <div className="agent-template-panel__advanced-fields">
                    <TextField
                      label={t("flow.agentEditor.toolsCsv")}
                      value={form.tools}
                      onChange={(event) =>
                        setFormValue(setForm, "tools", event.target.value)
                      }
                    />
                    <TextField
                      label={t("flow.agentEditor.skillsCsv")}
                      value={form.skills}
                      onChange={(event) =>
                        setFormValue(setForm, "skills", event.target.value)
                      }
                    />
                    <TextField
                      label={t("flow.agentEditor.pluginsCsv")}
                      value={form.plugins}
                      onChange={(event) =>
                        setFormValue(setForm, "plugins", event.target.value)
                      }
                    />
                    <TextField
                      label={t("flow.agentEditor.workspaceRootsCsv")}
                      value={form.workspaceRoots}
                      onChange={(event) =>
                        setFormValue(
                          setForm,
                          "workspaceRoots",
                          event.target.value,
                        )
                      }
                    />
                    <TextField
                      label={t("flow.agentEditor.models")}
                      value={form.models}
                      onChange={(event) =>
                        setFormValue(setForm, "models", event.target.value)
                      }
                    />
                    <TextAreaField
                      label={t("flow.agentEditor.resourceBindingsJson")}
                      value={form.resourceGrants}
                      onChange={(value) =>
                        setFormValue(setForm, "resourceGrants", value)
                      }
                      mono
                    />
                    <TextAreaField
                      label={t("flow.agentEditor.stateSchema")}
                      value={form.stateSchema}
                      onChange={(value) =>
                        setFormValue(setForm, "stateSchema", value)
                      }
                      mono
                    />
                    <TextAreaField
                      label={t("flow.agentEditor.outputSchema")}
                      value={form.outputSchema}
                      onChange={(value) =>
                        setFormValue(setForm, "outputSchema", value)
                      }
                      mono
                    />
                    <TextField
                      label={t("flow.agentEditor.delegatesCsv")}
                      value={form.delegates}
                      onChange={(event) =>
                        setFormValue(setForm, "delegates", event.target.value)
                      }
                    />
                  </div>
                </details>
              </div>
            </main>
            <AgentConfigInspector
              generating={busy === "generate"}
              preview={{
                name: form.name,
                instructions: form.instructions,
                connectionCount: form.connectionBindings.length,
                knowledge: agentKnowledgeBindingSummary(
                  form.knowledgeProvider
                    ? {
                        provider: form.knowledgeProvider,
                        namespaces:
                          form.knowledgeProvider === "sag"
                            ? parseAgentDraftList(form.knowledgeNamespaces)
                            : [],
                      }
                    : undefined,
                  language,
                ),
                riskClass: form.riskClass,
                tools: agentToolsWithKnowledgeAccess(
                  parseAgentDraftList(form.tools),
                  form.knowledgeProvider,
                ),
                outputSchema: form.outputSchema,
              }}
            />
          </div>
        </Panel>
      ) : null}

      {!editing && selected ? (
        <Panel
          title={`${selected.template.name} · v${selected.template.version}`}
          actions={
            <div className="agent-template-panel__header-actions">
              <Badge
                variant={
                  selected.template.status === "published"
                    ? "success"
                    : "warning"
                }
              >
                {selected.template.status === "published"
                  ? t("flow.agents.published")
                  : t("flow.agents.draft")}
              </Badge>
              <Badge variant={riskBadge(selected.template.spec.riskClass)}>
                {riskLabel(selected.template.spec.riskClass, language)}
              </Badge>
              {selected.template.status === "draft" ? (
                <Button
                  disabled={Boolean(busy)}
                  onClick={() => void publishSelected()}
                  size="compact"
                  variant="primary"
                >
                  {selected.diff.widensCapabilities ? (
                    <ShieldAlert size={14} aria-hidden="true" />
                  ) : null}
                  {t("flow.agentEditor.publish")}
                </Button>
              ) : (
                <Button
                  disabled={Boolean(busy)}
                  onClick={() => void archiveSelected()}
                  size="compact"
                  variant="quiet"
                >
                  {t("flow.agentEditor.archive")}
                </Button>
              )}
            </div>
          }
        >
          <dl className="agent-template-panel__facts">
            <div>
              <dt>{t("flow.agentEditor.owner")}</dt>
              <dd>{selected.template.owner}</dd>
            </div>
            <div>
              <dt>{t("flow.agentEditor.connections")}</dt>
              <dd>
                {selected.template.spec.connectionBindings?.length
                  ? `${selected.template.spec.connectionBindings.length} ${t("flow.agentEditor.structuredBindings")}`
                  : selected.template.spec.capabilities.mcpServers.length
                    ? `${selected.template.spec.capabilities.mcpServers.length} ${t("flow.agentEditor.legacyBindings")}`
                    : t("flow.agentEditor.noneValue")}
              </dd>
            </div>
            <div>
              <dt>{t("flow.agentEditor.knowledge")}</dt>
              <dd>
                {agentKnowledgeBindingSummary(
                  selected.template.spec.knowledgeBinding,
                  language,
                )}
              </dd>
            </div>
            <div>
              <dt>{t("flow.agentEditor.model")}</dt>
              <dd>
                {selected.template.spec.modelPolicy.allowedModels
                  .map((model) => `${model.providerId}:${model.modelId}`)
                  .join(", ") || t("flow.agentEditor.noneValue")}
              </dd>
            </div>
          </dl>
          <details className="agent-template-panel__technical">
            <summary>{t("flow.agentEditor.technical")}</summary>
            <dl className="agent-template-panel__facts">
              <div>
                <dt>{t("flow.agentEditor.contentHash")}</dt>
                <dd className="is-mono">{selected.template.contentHash}</dd>
              </div>
              <div>
                <dt>{t("flow.agentEditor.tools")}</dt>
                <dd>
                  {capabilitySummary(
                    selected.template.spec.capabilities.tools,
                    language,
                  )}
                </dd>
              </div>
              <div>
                <dt>{t("flow.agentEditor.skills")}</dt>
                <dd>
                  {capabilitySummary(
                    selected.template.spec.capabilities.skills,
                    language,
                  )}
                </dd>
              </div>
              <div>
                <dt>{t("flow.agentEditor.directories")}</dt>
                <dd>
                  {capabilitySummary(
                    selected.template.spec.capabilities.workspaceRoots,
                    language,
                  )}
                </dd>
              </div>
            </dl>
            <AgentTemplateConnectionAccessSummary
              access={connectionAccess}
              error={connectionAccessError}
              loading={connectionAccessLoading}
              onRetry={() =>
                setConnectionAccessRefresh((current) => current + 1)
              }
            />
          </details>
          {selected.template.status === "draft" ||
          selected.diff.changes.length ? (
            <div className="agent-template-panel__diff">
              <div className="agent-template-panel__section-title">
                <GitCompareArrows size={14} aria-hidden="true" />
                {t("flow.agentEditor.permissionDiff")}
                {selected.diff.widensCapabilities ? (
                  <Badge variant="warning">
                    {t("flow.agentEditor.widens")}
                  </Badge>
                ) : (
                  <Badge variant="success">
                    {t("flow.agentEditor.noWidens")}
                  </Badge>
                )}
              </div>
              {selected.diff.changes.length ? (
                <ul>
                  {selected.diff.changes.map((change, index) => (
                    <li
                      key={`${change.scope}:${change.value}:${change.kind}:${index}`}
                    >
                      <Badge
                        variant={
                          change.kind === "added" || change.kind === "expanded"
                            ? "warning"
                            : "neutral"
                        }
                      >
                        {changeKindLabel(change.kind, language)}
                      </Badge>
                      <span>{change.scope}</span>
                      <code>{change.value}</code>
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="agent-template-panel__empty">
                  {t("flow.agentEditor.noPermissionChanges")}
                </p>
              )}
            </div>
          ) : null}
          <div className="agent-template-panel__actions">
            <Button
              variant="quiet"
              disabled={Boolean(busy)}
              onClick={() => startNewVersion(selected)}
            >
              {t("flow.agentEditor.newFrom")}
            </Button>
            {selected.template.status === "draft" ? (
              <Button
                variant="danger"
                disabled={Boolean(busy)}
                onClick={() => void deleteSelected()}
              >
                <Trash2 size={14} aria-hidden="true" />
                {t("flow.agentEditor.deleteDraft")}
              </Button>
            ) : null}
          </div>
        </Panel>
      ) : null}

      {!editing ? (
        <Panel
          title={t("flow.agentEditor.currentSession")}
          actions={
            boundInstance ? (
              <Badge variant="success">{t("flow.agentEditor.bound")}</Badge>
            ) : (
              <Badge>{t("flow.agentEditor.unbound")}</Badge>
            )
          }
        >
          {selected?.template.status === "published" ? (
            <div className="agent-template-panel__instantiate">
              <details className="agent-template-panel__technical">
                <summary>{t("flow.agentEditor.initialState")}</summary>
                <TextAreaField
                  label={t("flow.agentEditor.initialStateJson")}
                  value={initialState}
                  onChange={setInitialState}
                  mono
                />
              </details>
              <Button
                variant="primary"
                disabled={!threadId || Boolean(busy)}
                onClick={() => void instantiateSelected()}
              >
                <UserRoundCog size={14} aria-hidden="true" />
                {t("flow.agentEditor.instantiate")}
              </Button>
            </div>
          ) : (
            <p className="agent-template-panel__empty">
              {t("flow.agentEditor.selectPublished")}
            </p>
          )}
          {instances.length ? (
            <details className="agent-template-panel__technical">
              <summary>
                {instances.length} {t("flow.agentEditor.instances")}
              </summary>
              <div className="agent-template-panel__instances">
                {instances.map((instance) => (
                  <article
                    key={instance.id}
                    className="agent-template-panel__instance"
                  >
                    <div>
                      <strong>
                        {instance.templateId}@{instance.templateVersion}
                      </strong>
                      <small>
                        {shortId(instance.id)} ·{" "}
                        {t("flow.agentEditor.stateRevision")}{" "}
                        {instance.stateRevision}
                      </small>
                    </div>
                    <Badge
                      variant={
                        instance.status === "active"
                          ? "success"
                          : instance.status === "revoked"
                            ? "danger"
                            : "neutral"
                      }
                    >
                      {instanceStatusLabel(instance.status, language)}
                    </Badge>
                    <div className="agent-template-panel__instance-actions">
                      {instance.status === "active" &&
                      !instance.parentInstanceId &&
                      boundInstance?.id !== instance.id ? (
                        <Button
                          size="compact"
                          variant="quiet"
                          disabled={Boolean(busy)}
                          onClick={() => void bindInstance(instance.id)}
                        >
                          {t("flow.agentEditor.bind")}
                        </Button>
                      ) : null}
                      {instance.status === "active" ? (
                        <Button
                          size="compact"
                          variant="quiet"
                          disabled={Boolean(busy)}
                          onClick={() => void revokeInstance(instance.id)}
                        >
                          {t("flow.agentEditor.revoke")}
                        </Button>
                      ) : null}
                    </div>
                  </article>
                ))}
              </div>
            </details>
          ) : null}
        </Panel>
      ) : null}

      {error ? (
        <p className="agent-template-panel__message is-error" role="alert">
          {error}
        </p>
      ) : null}
      {notice ? (
        <p className="agent-template-panel__message is-success" role="status">
          {notice}
        </p>
      ) : null}
    </div>
  );
}

function templateKey(view: AgentTemplateVersionView): string {
  return `${view.template.templateId}@${view.template.version}`;
}

function readableError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
