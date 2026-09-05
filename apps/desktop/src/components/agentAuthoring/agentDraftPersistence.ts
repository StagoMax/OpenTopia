import type { AgentConnectionBinding } from "../../types";
import type { AgentDraftForm } from "./agentDraftForm";

export type StoredAgentDraft = {
  form: AgentDraftForm;
  updatedAt: number;
};

const storageKey = "opentopia.flow-agent-drafts.v1";
const unassignedDraftKey = "workspace:unassigned";
const maxStoredDrafts = 25;
const stringFields = [
  "templateId",
  "name",
  "owner",
  "description",
  "instructions",
  "tools",
  "skills",
  "plugins",
  "mcpServers",
  "knowledgeNamespaces",
  "workspaceRoots",
  "models",
  "resourceGrants",
  "stateSchema",
  "outputSchema",
  "delegates",
] as const;
const knowledgeProviders = new Set(["", "sag", "graph-rag"]);
const riskClasses = new Set(["low", "medium", "high", "critical"]);

export function agentDraftStorageKey(workspaceRoot: string | null): string {
  const normalized = workspaceRoot
    ?.trim()
    .replaceAll("\\", "/")
    .replace(/\/+$/, "");
  return normalized ? `workspace:${normalized}` : unassignedDraftKey;
}

export function readAgentDraft(
  workspaceRoot: string | null,
  fallback: AgentDraftForm,
): StoredAgentDraft | null {
  if (typeof window === "undefined") return null;
  try {
    const drafts = parseDraftCollection(
      JSON.parse(window.localStorage.getItem(storageKey) ?? "{}"),
    );
    return parseStoredDraft(
      drafts[agentDraftStorageKey(workspaceRoot)],
      fallback,
    );
  } catch {
    return null;
  }
}

export function writeAgentDraft(
  workspaceRoot: string | null,
  form: AgentDraftForm,
): boolean {
  if (typeof window === "undefined") return false;
  try {
    const drafts = parseDraftCollection(
      JSON.parse(window.localStorage.getItem(storageKey) ?? "{}"),
    );
    drafts[agentDraftStorageKey(workspaceRoot)] = {
      form,
      updatedAt: Date.now(),
    };
    const entries = Object.entries(drafts)
      .sort(
        (left, right) => storedUpdatedAt(right[1]) - storedUpdatedAt(left[1]),
      )
      .slice(0, maxStoredDrafts);
    window.localStorage.setItem(
      storageKey,
      JSON.stringify(Object.fromEntries(entries)),
    );
    return true;
  } catch {
    return false;
  }
}

export function clearAgentDraft(workspaceRoot: string | null): void {
  if (typeof window === "undefined") return;
  try {
    const drafts = parseDraftCollection(
      JSON.parse(window.localStorage.getItem(storageKey) ?? "{}"),
    );
    delete drafts[agentDraftStorageKey(workspaceRoot)];
    if (Object.keys(drafts).length === 0) {
      window.localStorage.removeItem(storageKey);
    } else {
      window.localStorage.setItem(storageKey, JSON.stringify(drafts));
    }
  } catch {
    // A corrupt or unavailable store should not block the server save flow.
  }
}

export function parseStoredDraft(
  value: unknown,
  fallback: AgentDraftForm,
): StoredAgentDraft | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const stored = value as Record<string, unknown>;
  if (
    !stored.form ||
    typeof stored.form !== "object" ||
    Array.isArray(stored.form) ||
    typeof stored.updatedAt !== "number" ||
    !Number.isFinite(stored.updatedAt) ||
    stored.updatedAt < 0
  ) {
    return null;
  }

  const rawForm = stored.form as Record<string, unknown>;
  const form = { ...fallback };
  for (const field of stringFields) {
    const value = rawForm[field];
    if (value === undefined) continue;
    if (typeof value !== "string") return null;
    form[field] = value;
  }

  if (rawForm.legacyAllowAllMcpServers !== undefined) {
    if (typeof rawForm.legacyAllowAllMcpServers !== "boolean") return null;
    form.legacyAllowAllMcpServers = rawForm.legacyAllowAllMcpServers;
  }
  if (rawForm.knowledgeProvider !== undefined) {
    if (
      typeof rawForm.knowledgeProvider !== "string" ||
      !knowledgeProviders.has(rawForm.knowledgeProvider)
    ) {
      return null;
    }
    form.knowledgeProvider =
      rawForm.knowledgeProvider as AgentDraftForm["knowledgeProvider"];
  }
  if (rawForm.riskClass !== undefined) {
    if (
      typeof rawForm.riskClass !== "string" ||
      !riskClasses.has(rawForm.riskClass)
    ) {
      return null;
    }
    form.riskClass = rawForm.riskClass as AgentDraftForm["riskClass"];
  }
  if (rawForm.connectionBindings !== undefined) {
    const bindings = parseConnectionBindings(rawForm.connectionBindings);
    if (!bindings) return null;
    form.connectionBindings = bindings;
  }

  return { form, updatedAt: stored.updatedAt };
}

function parseDraftCollection(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? { ...(value as Record<string, unknown>) }
    : {};
}

function parseConnectionBindings(
  value: unknown,
): AgentConnectionBinding[] | null {
  if (!Array.isArray(value)) return null;
  const bindings: AgentConnectionBinding[] = [];
  for (const item of value) {
    if (!item || typeof item !== "object" || Array.isArray(item)) return null;
    const binding = item as Record<string, unknown>;
    if (
      typeof binding.connectionId !== "string" ||
      !binding.connectionId ||
      typeof binding.capabilityRevision !== "number" ||
      !Number.isInteger(binding.capabilityRevision) ||
      binding.capabilityRevision < 0 ||
      !Array.isArray(binding.operationGrants)
    ) {
      return null;
    }
    const operationIds: string[] = [];
    for (const item of binding.operationGrants) {
      if (!item || typeof item !== "object" || Array.isArray(item)) return null;
      const operationId = (item as Record<string, unknown>).operationId;
      if (typeof operationId !== "string" || !operationId) return null;
      operationIds.push(operationId);
    }
    bindings.push({
      connectionId: binding.connectionId,
      capabilityRevision: binding.capabilityRevision,
      operationGrants: [...new Set(operationIds)].map((operationId) => ({
        operationId,
      })),
    });
  }
  return bindings;
}

function storedUpdatedAt(value: unknown): number {
  if (!value || typeof value !== "object" || Array.isArray(value)) return 0;
  const updatedAt = (value as Record<string, unknown>).updatedAt;
  return typeof updatedAt === "number" && Number.isFinite(updatedAt)
    ? updatedAt
    : 0;
}
