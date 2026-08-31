import type { LibraryProviderId } from "./types";

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
