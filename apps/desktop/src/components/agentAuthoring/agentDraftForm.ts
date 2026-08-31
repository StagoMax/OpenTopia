import type { Dispatch, SetStateAction } from "react";
import type {
  AgentConnectionBinding,
  AgentTemplateSpec,
  AgentTemplateVersionView,
  AppSettings,
  LibraryProviderId,
} from "../../types";
import { normalizeConnectionBindings } from "../agentTemplateConnectionGrants";

export type AgentDraftForm = {
  templateId: string;
  name: string;
  owner: string;
  description: string;
  instructions: string;
  tools: string;
  skills: string;
  plugins: string;
  legacyAllowAllMcpServers: boolean;
  mcpServers: string;
  connectionBindings: AgentConnectionBinding[];
  knowledgeProvider: "" | LibraryProviderId;
  knowledgeNamespaces: string;
  workspaceRoots: string;
  models: string;
  resourceGrants: string;
  stateSchema: string;
  outputSchema: string;
  delegates: string;
  riskClass: AgentTemplateSpec["riskClass"];
};

export function blankAgentDraft(
  workspaceRoot: string | null,
  settings: AppSettings | null,
): AgentDraftForm {
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
    tools: "filesystem, shell, list_skills, read_skill",
    skills: "",
    plugins: "",
    legacyAllowAllMcpServers: false,
    mcpServers: "",
    connectionBindings: [],
    knowledgeProvider: "",
    knowledgeNamespaces: "",
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

export function agentDraftFromTemplate(
  view: AgentTemplateVersionView,
  workspaceRoot: string | null,
  settings: AppSettings | null,
): AgentDraftForm {
  const template = view.template;
  const fallback = blankAgentDraft(workspaceRoot, settings);
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
    legacyAllowAllMcpServers: template.spec.capabilities.allowAllMcpServers,
    mcpServers: template.spec.capabilities.mcpServers.join(", "),
    connectionBindings: normalizeConnectionBindings(
      template.spec.connectionBindings,
    ),
    knowledgeProvider: template.spec.knowledgeBinding
      ? (template.spec.knowledgeBinding.provider ?? "sag")
      : "",
    knowledgeNamespaces:
      template.spec.knowledgeBinding?.namespaces.join(", ") ?? "",
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

export function setAgentDraftValue<K extends keyof AgentDraftForm>(
  setForm: Dispatch<SetStateAction<AgentDraftForm>>,
  key: K,
  value: AgentDraftForm[K],
) {
  setForm((current) => ({ ...current, [key]: value }));
}

export function parseAgentDraftList(value: string): string[] {
  return [
    ...new Set(
      value
        .split(/[\n,]/)
        .map((item) => item.trim())
        .filter(Boolean),
    ),
  ];
}

export function parseAgentModelBindings(value: string) {
  return parseAgentDraftList(value).map((binding) => {
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

export function parseAgentDraftJson<T>(value: string, label: string): T {
  try {
    return JSON.parse(value) as T;
  } catch {
    throw new Error(`${label} 不是有效 JSON`);
  }
}
