import {
  useCallback,
  useEffect,
  useMemo,
  useState,
  type Dispatch,
  type SetStateAction,
} from "react";
import {
  Bot,
  GitCompareArrows,
  Plus,
  RefreshCw,
  ShieldAlert,
  Trash2,
  UserRoundCog,
} from "lucide-react";
import type { ApiClient } from "../api/client";
import type {
  AgentInstance,
  AgentTemplateSpec,
  AgentTemplateVersionView,
  AppSettings,
  ExecutionResourceGrant,
} from "../types";
import { Badge, Button, Panel, TextField } from "./ui";
import "../styles/agent-template-panel.css";

type AgentTemplatePanelProps = {
  client: ApiClient | null;
  threadId: string | null;
  workspaceRoot: string | null;
  settings: AppSettings | null;
};

type DraftForm = {
  templateId: string;
  name: string;
  owner: string;
  description: string;
  instructions: string;
  tools: string;
  skills: string;
  plugins: string;
  mcpServers: string;
  workspaceRoots: string;
  models: string;
  resourceGrants: string;
  stateSchema: string;
  outputSchema: string;
  delegates: string;
  riskClass: AgentTemplateSpec["riskClass"];
};

export function AgentTemplatePanel({
  client,
  threadId,
  workspaceRoot,
  settings,
}: AgentTemplatePanelProps) {
  const [templates, setTemplates] = useState<AgentTemplateVersionView[]>([]);
  const [instances, setInstances] = useState<AgentInstance[]>([]);
  const [boundInstance, setBoundInstance] = useState<AgentInstance | null>(
    null,
  );
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [form, setForm] = useState<DraftForm>(() =>
    blankDraft(workspaceRoot, settings),
  );
  const [editing, setEditing] = useState(false);
  const [initialState, setInitialState] = useState("{}");
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const selected = useMemo(
    () => templates.find((view) => templateKey(view) === selectedKey) ?? null,
    [selectedKey, templates],
  );

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
      setSelectedKey((current) => {
        if (
          current &&
          nextTemplates.some((view) => templateKey(view) === current)
        ) {
          return current;
        }
        return nextTemplates[0] ? templateKey(nextTemplates[0]) : null;
      });
    } catch (refreshError) {
      setError(readableError(refreshError));
    }
  }, [client, threadId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    if (!editing) setForm(blankDraft(workspaceRoot, settings));
  }, [editing, settings, workspaceRoot]);

  async function createVersion() {
    if (!client || busy) return;
    setBusy("create");
    setError(null);
    setNotice(null);
    try {
      const resourceGrants = parseJson<ExecutionResourceGrant[]>(
        form.resourceGrants,
        "资源绑定",
      );
      if (!Array.isArray(resourceGrants)) {
        throw new Error("资源绑定必须是 JSON 数组");
      }
      const stateSchema = parseJson<unknown>(form.stateSchema, "状态 Schema");
      const outputSchema = parseJson<unknown>(form.outputSchema, "输出 Schema");
      const created = await client.createAgentTemplateVersion({
        templateId: form.templateId.trim(),
        name: form.name.trim(),
        owner: form.owner.trim(),
        spec: {
          description: form.description.trim(),
          instructions: form.instructions.trim(),
          capabilities: {
            allowAllTools: false,
            tools: parseList(form.tools),
            allowAllSkills: false,
            skills: parseList(form.skills),
            allowAllPlugins: false,
            plugins: parseList(form.plugins),
            allowAllMcpServers: false,
            mcpServers: parseList(form.mcpServers),
            allowAllWorkspaceRoots: false,
            workspaceRoots: parseList(form.workspaceRoots),
          },
          resourceGrants,
          modelPolicy: {
            allowAllModels: false,
            allowedModels: parseModelBindings(form.models),
          },
          stateSchema,
          outputSchema,
          allowAllDelegates: false,
          delegateTemplateIds: parseList(form.delegates),
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
      await refresh();
      setSelectedKey(templateKey(created));
    } catch (createError) {
      setError(readableError(createError));
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
      await refresh();
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
      await refresh();
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
      await refresh();
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
      const state = parseJson<unknown>(initialState, "实例状态");
      const response = await client.createAgentInstance({
        templateId: selected.template.templateId,
        templateVersion: selected.template.version,
        threadId,
        initialState: state,
        bindToThread: true,
      });
      setNotice(`已创建并绑定 Agent ${shortId(response.instance.id)}`);
      await refresh();
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
      await refresh();
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
      await refresh();
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
        ? draftFromTemplate(source, workspaceRoot, settings)
        : blankDraft(workspaceRoot, settings),
    );
    setEditing(true);
  }

  return (
    <div className="agent-template-panel" aria-label="Agent 模板与身份">
      <Panel
        title="Agent 模板"
        actions={
          <div className="agent-template-panel__header-actions">
            <Button
              size="compact"
              variant="quiet"
              aria-label="刷新 Agent 模板"
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
          <p className="agent-template-panel__empty">尚未创建 Agent 模板。</p>
        )}
      </Panel>

      {editing ? (
        <Panel title="创建不可变版本">
          <div className="agent-template-panel__form">
            <TextField
              label="模板 ID"
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
              label="身份指令"
              value={form.instructions}
              onChange={(value) => setFormValue(setForm, "instructions", value)}
            />
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
              label="MCP Server ID（逗号分隔）"
              value={form.mcpServers}
              onChange={(event) =>
                setFormValue(setForm, "mcpServers", event.target.value)
              }
            />
            <TextField
              label="工作目录（逗号分隔）"
              value={form.workspaceRoots}
              onChange={(event) =>
                setFormValue(setForm, "workspaceRoots", event.target.value)
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
              onChange={(value) => setFormValue(setForm, "stateSchema", value)}
              mono
            />
            <TextAreaField
              label="输出 Schema"
              value={form.outputSchema}
              onChange={(value) => setFormValue(setForm, "outputSchema", value)}
              mono
            />
            <TextField
              label="可委派模板（逗号分隔）"
              value={form.delegates}
              onChange={(event) =>
                setFormValue(setForm, "delegates", event.target.value)
              }
            />
            <label className="agent-template-panel__field">
              <span>风险等级</span>
              <select
                value={form.riskClass}
                onChange={(event) =>
                  setFormValue(
                    setForm,
                    "riskClass",
                    event.target.value as DraftForm["riskClass"],
                  )
                }
              >
                <option value="low">低</option>
                <option value="medium">中</option>
                <option value="high">高</option>
                <option value="critical">关键</option>
              </select>
            </label>
            <div className="agent-template-panel__actions">
              <Button
                variant="quiet"
                disabled={Boolean(busy)}
                onClick={() => setEditing(false)}
              >
                取消
              </Button>
              <Button
                variant="primary"
                disabled={!client || busy === "create"}
                onClick={() => void createVersion()}
              >
                {busy === "create" ? "创建中…" : "创建版本"}
              </Button>
            </div>
          </div>
        </Panel>
      ) : null}

      {selected ? (
        <Panel
          title={`${selected.template.name} · v${selected.template.version}`}
          actions={
            <Badge variant={riskBadge(selected.template.spec.riskClass)}>
              {riskLabel(selected.template.spec.riskClass)}
            </Badge>
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
              <dt>模型</dt>
              <dd>
                {selected.template.spec.modelPolicy.allowedModels
                  .map((model) => `${model.providerId}:${model.modelId}`)
                  .join(", ") || "无"}
              </dd>
            </div>
          </dl>
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
              <>
                <Button
                  variant="danger"
                  disabled={Boolean(busy)}
                  onClick={() => void deleteSelected()}
                >
                  <Trash2 size={14} aria-hidden="true" />
                  删除草稿
                </Button>
                <Button
                  variant="primary"
                  disabled={Boolean(busy)}
                  onClick={() => void publishSelected()}
                >
                  {selected.diff.widensCapabilities ? (
                    <ShieldAlert size={14} aria-hidden="true" />
                  ) : null}
                  发布并锁定
                </Button>
              </>
            ) : (
              <Button
                variant="quiet"
                disabled={Boolean(busy)}
                onClick={() => void archiveSelected()}
              >
                归档模板
              </Button>
            )}
          </div>
        </Panel>
      ) : null}

      <Panel
        title="当前会话 Agent"
        actions={
          boundInstance ? (
            <Badge variant="success">已绑定 {shortId(boundInstance.id)}</Badge>
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

function blankDraft(
  workspaceRoot: string | null,
  settings: AppSettings | null,
): DraftForm {
  const provider = settings?.providers.find(
    (item) => item.id === settings.activeProviderId,
  );
  return {
    templateId: "",
    name: "",
    owner: "enterprise-admin",
    description: "",
    instructions:
      "只在当前 ExecutionContext 投影的能力范围内完成任务；无法确定时明确标记 unknown。",
    tools:
      "read_file, read_files, search, git_diff, list_skills, read_skill, complete_task",
    skills: "",
    plugins: "",
    mcpServers: "",
    workspaceRoots: workspaceRoot ?? "",
    models: provider ? `${provider.id}:${provider.model}` : "",
    resourceGrants: "[]",
    stateSchema:
      '{"type":"object","properties":{},"additionalProperties":false}',
    outputSchema: '{"type":"object"}',
    delegates: "",
    riskClass: "medium",
  };
}

function draftFromTemplate(
  view: AgentTemplateVersionView,
  workspaceRoot: string | null,
  settings: AppSettings | null,
): DraftForm {
  const template = view.template;
  const fallback = blankDraft(workspaceRoot, settings);
  return {
    ...fallback,
    templateId: template.templateId,
    name: template.name,
    owner: template.owner,
    description: template.spec.description,
    instructions: template.spec.instructions,
    tools: template.spec.capabilities.tools.join(", "),
    skills: template.spec.capabilities.skills.join(", "),
    plugins: template.spec.capabilities.plugins.join(", "),
    mcpServers: template.spec.capabilities.mcpServers.join(", "),
    workspaceRoots: template.spec.capabilities.workspaceRoots.join(", "),
    models: template.spec.modelPolicy.allowedModels
      .map((model) => `${model.providerId}:${model.modelId}`)
      .join(", "),
    resourceGrants: JSON.stringify(template.spec.resourceGrants, null, 2),
    stateSchema: JSON.stringify(template.spec.stateSchema, null, 2),
    outputSchema: JSON.stringify(template.spec.outputSchema, null, 2),
    delegates: template.spec.delegateTemplateIds.join(", "),
    riskClass: template.spec.riskClass,
  };
}

function TextAreaField({
  label,
  value,
  onChange,
  mono = false,
}: {
  label: string;
  value: string;
  onChange(value: string): void;
  mono?: boolean;
}) {
  return (
    <label className="agent-template-panel__field">
      <span>{label}</span>
      <textarea
        className={mono ? "is-mono" : undefined}
        value={value}
        onChange={(event) => onChange(event.target.value)}
      />
    </label>
  );
}

function setFormValue<K extends keyof DraftForm>(
  setForm: Dispatch<SetStateAction<DraftForm>>,
  key: K,
  value: DraftForm[K],
) {
  setForm((current) => ({ ...current, [key]: value }));
}

function parseList(value: string): string[] {
  return [
    ...new Set(
      value
        .split(/[\n,]/)
        .map((item) => item.trim())
        .filter(Boolean),
    ),
  ];
}

function parseModelBindings(value: string) {
  return parseList(value).map((binding) => {
    const separator = binding.indexOf(":");
    if (separator <= 0 || separator === binding.length - 1) {
      throw new Error(`模型绑定格式无效：${binding}`);
    }
    return {
      providerId: binding.slice(0, separator),
      modelId: binding.slice(separator + 1),
    };
  });
}

function parseJson<T>(value: string, label: string): T {
  try {
    return JSON.parse(value) as T;
  } catch {
    throw new Error(`${label} 不是有效 JSON`);
  }
}

function templateKey(view: AgentTemplateVersionView): string {
  return `${view.template.templateId}@${view.template.version}`;
}

function shortId(value: string): string {
  return value.slice(0, 8);
}

function readableError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function capabilitySummary(values: string[]): string {
  return values.length ? values.join(", ") : "无";
}

function riskLabel(risk: AgentTemplateSpec["riskClass"]): string {
  return {
    low: "低风险",
    medium: "中风险",
    high: "高风险",
    critical: "关键风险",
  }[risk];
}

function riskBadge(
  risk: AgentTemplateSpec["riskClass"],
): "neutral" | "warning" | "danger" {
  if (risk === "critical") return "danger";
  if (risk === "high") return "warning";
  return "neutral";
}

function changeKindLabel(
  kind: "added" | "removed" | "expanded" | "reduced",
): string {
  return { added: "新增", removed: "移除", expanded: "扩展", reduced: "收窄" }[
    kind
  ];
}

function instanceStatusLabel(status: AgentInstance["status"]): string {
  return {
    active: "运行中",
    suspended: "已暂停",
    completed: "已完成",
    revoked: "已撤销",
  }[status];
}
