import type { LibraryProviderId } from "./types";

export type FlowLibraryProviderSelection = "" | LibraryProviderId;

export const FLOW_LIBRARY_PROVIDER_OPTIONS = [
  { value: "", label: "不启用资料库" },
  { value: "graph-rag", label: "Graph RAG" },
  { value: "sag", label: "SAG" },
] as const;

export function flowLibraryProviderLabel(
  provider: LibraryProviderId | null | undefined,
): string {
  if (provider === "graph-rag") return "Graph RAG";
  if (provider === "sag") return "SAG";
  return "未启用";
}

export function resolveFlowLibraryProvider(
  threadId: string | null,
  bindings: Readonly<Record<string, LibraryProviderId>>,
  draftProvider: LibraryProviderId | null,
): LibraryProviderId | null {
  return threadId ? (bindings[threadId] ?? null) : draftProvider;
}

export function updateFlowLibraryBindings(
  bindings: Readonly<Record<string, LibraryProviderId>>,
  threadId: string,
  provider: LibraryProviderId | null,
): Record<string, LibraryProviderId> {
  if (provider) {
    if (bindings[threadId] === provider) return bindings;
    return { ...bindings, [threadId]: provider };
  }
  if (!(threadId in bindings)) return bindings;
  const next = { ...bindings };
  delete next[threadId];
  return next;
}
