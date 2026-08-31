import type { AgentKnowledgeBinding, LibraryProviderId } from "./types";

export type AgentKnowledgeProviderSelection = "" | LibraryProviderId;

export const AGENT_KNOWLEDGE_PROVIDER_OPTIONS = [
  { value: "", label: "不使用知识库" },
  { value: "graph-rag", label: "Graph RAG" },
  { value: "sag", label: "SAG" },
] as const;

export function agentKnowledgeProvider(
  binding: AgentKnowledgeBinding | null | undefined,
): LibraryProviderId | null {
  return binding ? (binding.provider ?? "sag") : null;
}

export function agentKnowledgeProviderLabel(
  bindingOrProvider:
    AgentKnowledgeBinding | LibraryProviderId | null | undefined,
): string {
  const provider =
    typeof bindingOrProvider === "string"
      ? bindingOrProvider
      : agentKnowledgeProvider(bindingOrProvider);
  if (provider === "graph-rag") return "Graph RAG";
  if (provider === "sag") return "SAG";
  return "未绑定";
}

export function agentKnowledgeBindingSummary(
  binding: AgentKnowledgeBinding | null | undefined,
): string {
  const provider = agentKnowledgeProvider(binding);
  if (!provider) return "未绑定";
  if (provider === "graph-rag") return "Graph RAG";
  return `SAG · ${binding?.namespaces.join(", ") || "未配置 namespace"}`;
}

export function agentToolsWithKnowledgeAccess(
  tools: readonly string[],
  provider: AgentKnowledgeProviderSelection,
): string[] {
  return provider && !tools.includes("library_search")
    ? [...tools, "library_search"]
    : [...tools];
}
