import type { AgentKnowledgeBinding, LibraryProviderId } from "./types";
import {
  defaultApplicationLanguage,
  interfaceMessage,
  type ApplicationLanguage,
} from "./applicationLanguage.ts";

export type AgentKnowledgeProviderSelection = "" | LibraryProviderId;

export const AGENT_KNOWLEDGE_PROVIDER_OPTIONS = [
  { value: "", label: "不使用知识库" },
  { value: "graph-rag", label: "Graph RAG" },
  { value: "sag", label: "SAG" },
] as const;

export function agentKnowledgeProviderOptions(
  language: ApplicationLanguage = defaultApplicationLanguage,
) {
  return [
    {
      value: "" as const,
      label: interfaceMessage(language, "flow.agentKnowledge.none"),
    },
    { value: "graph-rag" as const, label: "Graph RAG" },
    { value: "sag" as const, label: "SAG" },
  ];
}

export function agentKnowledgeProvider(
  binding: AgentKnowledgeBinding | null | undefined,
): LibraryProviderId | null {
  return binding ? (binding.provider ?? "sag") : null;
}

export function agentKnowledgeProviderLabel(
  bindingOrProvider:
    AgentKnowledgeBinding | LibraryProviderId | null | undefined,
  language: ApplicationLanguage = defaultApplicationLanguage,
): string {
  const provider =
    typeof bindingOrProvider === "string"
      ? bindingOrProvider
      : agentKnowledgeProvider(bindingOrProvider);
  if (provider === "graph-rag") return "Graph RAG";
  if (provider === "sag") return "SAG";
  return interfaceMessage(language, "flow.agentKnowledge.unbound");
}

export function agentKnowledgeBindingSummary(
  binding: AgentKnowledgeBinding | null | undefined,
  language: ApplicationLanguage = defaultApplicationLanguage,
): string {
  const provider = agentKnowledgeProvider(binding);
  if (!provider)
    return interfaceMessage(language, "flow.agentKnowledge.unbound");
  if (provider === "graph-rag") return "Graph RAG";
  return `SAG · ${binding?.namespaces.join(", ") || interfaceMessage(language, "flow.agentKnowledge.namespaceMissing")}`;
}

export function agentToolsWithKnowledgeAccess(
  tools: readonly string[],
  provider: AgentKnowledgeProviderSelection,
): string[] {
  return provider && !tools.includes("library_search")
    ? [...tools, "library_search"]
    : [...tools];
}
