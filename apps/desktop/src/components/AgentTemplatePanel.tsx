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
import type {
  AgentInstance,
  AgentTemplateVersionView,
  AppSettings,
  ExecutionResourceGrant,
} from "../types";
import type { AgentTemplateConnectionAccessView } from "../api/generated/desktop-http-v1.generated";
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
  const selection = useFlowAgentSelection();
  const [templates, setTemplates] = useState<AgentTemplateVersionView[]>([]);
  const [instances, setInstances] = useState<AgentInstance[]>([]);
  const [boundInstance, setBoundInstance] = useState<AgentInstance | null>(
    null,
  );
  const [localSelectedKey, setLocalSelectedKey] = useState<string | null>(null);
  const [form, setForm] = useState<AgentDraftForm>(() =>
    blankAgentDraft(workspaceRoot, settings),
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
    title: "Agents / 创建 Agent",
    backLabel: "返回 Agents",
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
    if (!editing) setForm(blankAgentDraft(workspaceRoot, settings));
  }, [editing, settings, workspaceRoot]);

  async function createVersion() {
    if (!client || busy) return;
    setBusy("create");
    setError(null);
    setNotice(null);
    try {
      const resourceGrants = parseAgentDraftJson<ExecutionResourceGrant[]>(
        form.resourceGrants,
        "资源绑定",
      );
      if (!Array.isArray(resourceGrants)) {
        throw new Error("资源绑定必须是 JSON 数组");
      }
      const stateSchema = parseAgentDraftJson<unknown>(
        form.stateSchema,
        "状态 Schema",
      );
      const outputSchema = parseAgentDraftJson<unknown>(
        form.outputSchema,
        "输出 Schema",
      );
      const knowledgeNamespaces = parseAgentDraftList(form.knowledgeNamespaces);
      if (form.knowledgeEnabled && knowledgeNamespaces.length === 0) {
        throw new Error("启用 SAG 知识绑定后，至少需要一个 namespace");
      }
      const tools = parseAgentDraftList(form.tools);
      if (form.knowledgeEnabled && !tools.includes("library_search")) {
        tools.push("library_search");
      }
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
          knowledgeBinding: form.knowledgeEnabled
            ? { namespaces: knowledgeNamespaces }
            : undefined,
          resourceGrants,
          modelPolicy: {
            allowAllModels: false,
            allowedModels: parseAgentModelBindings(form.models),
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
        `已创建 ${created.template.templateId}@${created.template.version}`,
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
      setForm(agentDraftFromTemplate(generated, workspaceRoot, settings));
      setSelectedKey(templateKey(generated));
      setNotice(
        `模型已生成 ${generated.template.name} 的实时配置；请在右侧审核后保存或直接返回列表发布。`,
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
      setNotice("版本已发布并锁定");
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
      setNotice("草稿版本已删除");
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
      setNotice("模板已归档；现有实例不会被自动扩权或重建");
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
      const state = parseAgentDraftJson<unknown>(initialState, "实例状态");
      const response = await client.createAgentInstance({
        templateId: selected.template.templateId,
        templateVersion: selected.template.version,
        threadId,
        initialState: state,
        bindToThread: true,
      });
      setNotice(`已创建并绑定 Agent ${shortId(response.instance.id)}`);
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
      setNotice(`已切换有效 Agent 为 ${shortId(instanceId)}`);
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
      setNotice(`已撤销 Agent ${shortId(instanceId)}`);
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
        ? agentDraftFromTemplate(source, workspaceRoot, settings)
        : blankAgentDraft(workspaceRoot, settings),
    );
    setRequirement(source?.template.spec.description ?? "");
    setEditing(true);
  }

  return (
    <div
      className={`agent-template-panel agent-template-panel--${variant}`}
      aria-label="Agent 配置与运行实例"
    >
      {!editing && showTemplateCollection ? (
        <Panel
          title="Agents / Agent 配置"
          actions={
            <div className="agent-template-panel__header-actions">
              <Badge variant="warning">Draft</Badge>
              <Button
                size="compact"
                variant="quiet"
                aria-label="刷新 Agents"
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
                新建
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
                      {view.template.status === "published" ? "已发布" : "草稿"}
                    </Badge>
                  </button>
                );
              })}
            </div>
          ) : (
            <p className="agent-template-panel__empty">尚未创建 Agent。</p>
          )}
        </Panel>
      ) : null}

      {editing ? (
        <Panel
          title="Create Agent / 创建 Agent"
          actions={
            <div className="agent-template-panel__header-actions">
              <Button
                disabled={Boolean(busy)}
                onClick={closeEditor}
                size="compact"
                variant="quiet"
              >
                取消
              </Button>
              <Button
                disabled={!client || busy === "create"}
                onClick={() => void createVersion()}
                size="compact"
                variant="primary"
              >
                {busy === "create" ? "保存中…" : "保存版本"}
              </Button>
            </div>
          }
        >
          <div className="agent-studio">
            <main className="agent-studio__main">
              <section className="agent-studio__composer">
                <span>
                  <strong>Describe the Agent / 描述你需要的 Agent</strong>
                  <small>
                    模型会通过受控的 agent_create 工具生成配置；Flow
                    创建不使用这条自然语言路径。
                  </small>
                </span>
                <textarea
                  onChange={(event) => setRequirement(event.target.value)}
                  placeholder="例如：当收到理赔案件参数时，查询案件详情和工伤政策知识库，输出结构化审核结论；金额异常时请求人工审批。"
                  value={requirement}
                />
                <div className="agent-studio__composer-actions">
                  <small>
                    生成过程运行在当前 Flow 会话中；如果权限策略要求审批，请在
                    Inbox 处理后继续。
                  </small>
                  <Button
                    disabled={!threadId || !requirement.trim() || Boolean(busy)}
                    onClick={() => void generateWithModel()}
                    variant="primary"
                  >
                    <Sparkles aria-hidden="true" size={14} />
                    {busy === "generate" ? "生成中…" : "生成 Agent 配置"}
                  </Button>
                </div>
              </section>
              <div className="agent-template-panel__form">
                <TextField
                  label="Agent ID"
                  value={form.templateId}
                  onChange={(event) =>
                    setFormValue(setForm, "templateId", event.target.value)
                  }
                />
                <TextField
                  label="名称"
                  value={form.name}
                  onChange={(event) =>
                    setFormValue(setForm, "name", event.target.value)
                  }
                />
                <TextField
                  label="所有者"
                  value={form.owner}
                  onChange={(event) =>
                    setFormValue(setForm, "owner", event.target.value)
                  }
                />
                <TextField
                  label="说明"
                  value={form.description}
                  onChange={(event) =>
                    setFormValue(setForm, "description", event.target.value)
                  }
                />
                <TextAreaField
                  label="Instructions / Agent 提示词"
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
                  enabled={form.knowledgeEnabled}
                  namespaces={form.knowledgeNamespaces}
                  onEnabledChange={(knowledgeEnabled) =>
                    setFormValue(setForm, "knowledgeEnabled", knowledgeEnabled)
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
                  label="风险等级"
                  value={form.riskClass}
                  onChange={(value) =>
                    setFormValue(
                      setForm,
                      "riskClass",
                      value as AgentDraftForm["riskClass"],
                    )
                  }
                  options={[
                    { value: "low", label: "低" },
                    { value: "medium", label: "中" },
                    { value: "high", label: "高" },
                    { value: "critical", label: "关键" },
                  ]}
                />
                <details className="agent-template-panel__advanced">
                  <summary>Advanced / 高级能力与 JSON Schema</summary>
                  <div className="agent-template-panel__advanced-fields">
                    <TextField
                      label="工具（逗号分隔）"
                      value={form.tools}
                      onChange={(event) =>
                        setFormValue(setForm, "tools", event.target.value)
                      }
                    />
                    <TextField
                      label="Skill（逗号分隔）"
                      value={form.skills}
                      onChange={(event) =>
                        setFormValue(setForm, "skills", event.target.value)
                      }
                    />
                    <TextField
                      label="插件（逗号分隔）"
                      value={form.plugins}
                      onChange={(event) =>
                        setFormValue(setForm, "plugins", event.target.value)
                      }
                    />
                    <TextField
                      label="工作目录（逗号分隔）"
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
                      label="模型（provider:model）"
                      value={form.models}
                      onChange={(event) =>
                        setFormValue(setForm, "models", event.target.value)
                      }
                    />
                    <TextAreaField
                      label="资源绑定 JSON"
                      value={form.resourceGrants}
                      onChange={(value) =>
                        setFormValue(setForm, "resourceGrants", value)
                      }
                      mono
                    />
                    <TextAreaField
                      label="状态 Schema"
                      value={form.stateSchema}
                      onChange={(value) =>
                        setFormValue(setForm, "stateSchema", value)
                      }
                      mono
                    />
                    <TextAreaField
                      label="输出 Schema"
                      value={form.outputSchema}
                      onChange={(value) =>
                        setFormValue(setForm, "outputSchema", value)
                      }
                      mono
                    />
                    <TextField
                      label="可委派 Agent（逗号分隔）"
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
                knowledgeNamespaces: form.knowledgeEnabled
                  ? parseAgentDraftList(form.knowledgeNamespaces)
                  : [],
                riskClass: form.riskClass,
                tools: parseAgentDraftList(form.tools),
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
                  ? "Published"
                  : "Draft"}
              </Badge>
              <Badge variant={riskBadge(selected.template.spec.riskClass)}>
                {riskLabel(selected.template.spec.riskClass)}
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
                  发布
                </Button>
              ) : (
                <Button
                  disabled={Boolean(busy)}
                  onClick={() => void archiveSelected()}
                  size="compact"
                  variant="quiet"
                >
                  归档
                </Button>
              )}
            </div>
          }
        >
          <dl className="agent-template-panel__facts">
            <div>
              <dt>所有者</dt>
              <dd>{selected.template.owner}</dd>
            </div>
            <div>
              <dt>内容哈希</dt>
              <dd className="is-mono">{selected.template.contentHash}</dd>
            </div>
            <div>
              <dt>工具</dt>
              <dd>
                {capabilitySummary(selected.template.spec.capabilities.tools)}
              </dd>
            </div>
            <div>
              <dt>Skill</dt>
              <dd>
                {capabilitySummary(selected.template.spec.capabilities.skills)}
              </dd>
            </div>
            <div>
              <dt>目录</dt>
              <dd>
                {capabilitySummary(
                  selected.template.spec.capabilities.workspaceRoots,
                )}
              </dd>
            </div>
            <div>
              <dt>Connections</dt>
              <dd>
                {selected.template.spec.connectionBindings?.length
                  ? `${selected.template.spec.connectionBindings.length} 个结构化绑定`
                  : selected.template.spec.capabilities.mcpServers.length
                    ? `${selected.template.spec.capabilities.mcpServers.length} 个 Legacy MCP 绑定`
                    : "无"}
              </dd>
            </div>
            <div>
              <dt>SAG 知识</dt>
              <dd>
                {selected.template.spec.knowledgeBinding?.namespaces.join(
                  ", ",
                ) || "无"}
              </dd>
            </div>
            <div>
              <dt>模型</dt>
              <dd>
                {selected.template.spec.modelPolicy.allowedModels
                  .map((model) => `${model.providerId}:${model.modelId}`)
                  .join(", ") || "无"}
              </dd>
            </div>
          </dl>
          <AgentTemplateConnectionAccessSummary
            access={connectionAccess}
            error={connectionAccessError}
            loading={connectionAccessLoading}
            onRetry={() => setConnectionAccessRefresh((current) => current + 1)}
          />
          <div className="agent-template-panel__diff">
            <div className="agent-template-panel__section-title">
              <GitCompareArrows size={14} aria-hidden="true" />
              权限差异
              {selected.diff.widensCapabilities ? (
                <Badge variant="warning">包含扩权</Badge>
              ) : (
                <Badge variant="success">未扩权</Badge>
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
                      {changeKindLabel(change.kind)}
                    </Badge>
                    <span>{change.scope}</span>
                    <code>{change.value}</code>
                  </li>
                ))}
              </ul>
            ) : (
              <p className="agent-template-panel__empty">
                与上一发布版本没有权限变化。
              </p>
            )}
          </div>
          <div className="agent-template-panel__actions">
            <Button
              variant="quiet"
              disabled={Boolean(busy)}
              onClick={() => startNewVersion(selected)}
            >
              基于此版本新建
            </Button>
            {selected.template.status === "draft" ? (
              <Button
                variant="danger"
                disabled={Boolean(busy)}
                onClick={() => void deleteSelected()}
              >
                <Trash2 size={14} aria-hidden="true" />
                删除草稿
              </Button>
            ) : null}
          </div>
        </Panel>
      ) : null}

      {!editing ? (
        <Panel
          title="当前会话 Agent"
          actions={
            boundInstance ? (
              <Badge variant="success">
                已绑定 {shortId(boundInstance.id)}
              </Badge>
            ) : (
              <Badge>未绑定</Badge>
            )
          }
        >
          {selected?.template.status === "published" ? (
            <div className="agent-template-panel__instantiate">
              <TextAreaField
                label="初始状态 JSON"
                value={initialState}
                onChange={setInitialState}
                mono
              />
              <Button
                variant="primary"
                disabled={!threadId || Boolean(busy)}
                onClick={() => void instantiateSelected()}
              >
                <UserRoundCog size={14} aria-hidden="true" />
                实例化并绑定
              </Button>
            </div>
          ) : (
            <p className="agent-template-panel__empty">
              选择一个已发布版本后可实例化。
            </p>
          )}
          {instances.length ? (
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
                      {shortId(instance.id)} · 状态修订 {instance.stateRevision}
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
                    {instanceStatusLabel(instance.status)}
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
                        绑定
                      </Button>
                    ) : null}
                    {instance.status === "active" ? (
                      <Button
                        size="compact"
                        variant="quiet"
                        disabled={Boolean(busy)}
                        onClick={() => void revokeInstance(instance.id)}
                      >
                        撤销
                      </Button>
                    ) : null}
                  </div>
                </article>
              ))}
            </div>
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
