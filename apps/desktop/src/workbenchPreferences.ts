import type {
  ExperienceMode,
  ProviderAdapterKind,
  ProviderKind,
  ReasoningEffort,
  ThreadModelSelection,
} from "./types";
import {
  reconcileReasoningEffort,
  resolveDefaultModelId,
} from "./modelCatalog.ts";

export type SidebarNavigationState = {
  expandedProjectIds: string[];
  unassignedExpanded: boolean;
  archivedExpanded: boolean;
  collapsed: boolean;
};

type LastActiveThreadIds = Partial<Record<ExperienceMode, string>>;

type DraftModelProvider = {
  id: string;
  kind: ProviderKind;
  model: string;
  enabledFamilies: string[];
  syncedModels: string[];
  reasoningEffort?: ReasoningEffort | null;
  allowedAdapters?: ProviderAdapterKind[];
  preferredAdapter?: ProviderAdapterKind | null;
};

const sidebarStorageKey = "opentopia.sidebar-navigation.v1";
const draftModelStorageKey = "opentopia.draft-model-selection.v1";
const lastActiveThreadStorageKey = "opentopia.last-active-thread.v1";

const defaultSidebarState: SidebarNavigationState = {
  expandedProjectIds: [],
  unassignedExpanded: false,
  archivedExpanded: false,
  collapsed: false,
};

const reasoningEfforts = new Set<ReasoningEffort>([
  "none",
  "minimal",
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
]);

const providerAdapters = new Set<ProviderAdapterKind>([
  "open_ai_chat",
  "open_ai_responses",
  "anthropic_messages",
  "codex_app_server",
  "mock",
]);

export function parseSidebarNavigationState(
  value: unknown,
): SidebarNavigationState {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return defaultSidebarState;
  }
  const stored = value as Record<string, unknown>;
  return {
    expandedProjectIds: Array.isArray(stored.expandedProjectIds)
      ? Array.from(
          new Set(
            stored.expandedProjectIds.filter(
              (projectId): projectId is string =>
                typeof projectId === "string" && projectId.length > 0,
            ),
          ),
        ).slice(0, 500)
      : [],
    unassignedExpanded:
      typeof stored.unassignedExpanded === "boolean"
        ? stored.unassignedExpanded
        : false,
    archivedExpanded:
      typeof stored.archivedExpanded === "boolean"
        ? stored.archivedExpanded
        : false,
    collapsed: typeof stored.collapsed === "boolean" ? stored.collapsed : false,
  };
}

export function readSidebarNavigationState(): SidebarNavigationState {
  if (typeof window === "undefined") return defaultSidebarState;
  try {
    return parseSidebarNavigationState(
      JSON.parse(window.localStorage.getItem(sidebarStorageKey) ?? "{}"),
    );
  } catch {
    return defaultSidebarState;
  }
}

export function updateSidebarNavigationState(
  patch: Partial<SidebarNavigationState>,
): void {
  if (typeof window === "undefined") return;
  try {
    const next = { ...readSidebarNavigationState(), ...patch };
    window.localStorage.setItem(sidebarStorageKey, JSON.stringify(next));
  } catch {
    // Sidebar state remains available for the current session if storage fails.
  }
}

export function parseDraftModelSelection(
  value: unknown,
): ThreadModelSelection | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const stored = value as Record<string, unknown>;
  if (
    typeof stored.connectionId !== "string" ||
    !stored.connectionId ||
    typeof stored.modelId !== "string" ||
    !stored.modelId ||
    !(
      stored.adapter === undefined ||
      stored.adapter === null ||
      (typeof stored.adapter === "string" &&
        providerAdapters.has(stored.adapter as ProviderAdapterKind))
    ) ||
    !(
      stored.reasoningEffort === null ||
      (typeof stored.reasoningEffort === "string" &&
        reasoningEfforts.has(stored.reasoningEffort as ReasoningEffort))
    )
  ) {
    return null;
  }
  const adapter =
    typeof stored.adapter === "string"
      ? (stored.adapter as ProviderAdapterKind)
      : null;
  return {
    connectionId: stored.connectionId,
    modelId: stored.modelId,
    ...(adapter ? { adapter } : {}),
    reasoningEffort: stored.reasoningEffort as ReasoningEffort | null,
  };
}

export function readDraftModelSelection(): ThreadModelSelection | null {
  if (typeof window === "undefined") return null;
  try {
    return parseDraftModelSelection(
      JSON.parse(window.localStorage.getItem(draftModelStorageKey) ?? "null"),
    );
  } catch {
    return null;
  }
}

export function writeDraftModelSelection(
  selection: ThreadModelSelection | null,
): void {
  if (typeof window === "undefined") return;
  try {
    if (selection) {
      window.localStorage.setItem(
        draftModelStorageKey,
        JSON.stringify(selection),
      );
    } else {
      window.localStorage.removeItem(draftModelStorageKey);
    }
  } catch {
    // The selection remains available for the current session if storage fails.
  }
}

export function resolveDraftModelSelection(
  providers: readonly DraftModelProvider[],
  activeProviderId: string,
  storedSelection: ThreadModelSelection | null,
): ThreadModelSelection | null {
  const activeProvider =
    providers.find((provider) => provider.id === activeProviderId) ??
    providers[0];
  if (!activeProvider) return null;
  const provider = activeProvider;
  const modelIds =
    provider.syncedModels.length > 0
      ? Array.from(new Set([...provider.syncedModels, provider.model]))
      : [provider.model];
  const canRestoreStored =
    storedSelection?.connectionId === provider.id &&
    modelIds.includes(storedSelection.modelId);
  const modelId = canRestoreStored
    ? storedSelection.modelId
    : resolveDefaultModelId(modelIds, provider.enabledFamilies, provider.model);
  const allowedAdapters = provider.allowedAdapters?.length
    ? provider.allowedAdapters
    : provider.kind === "anthropic"
      ? (["anthropic_messages"] as ProviderAdapterKind[])
      : provider.kind === "codex_app_server"
        ? (["codex_app_server"] as ProviderAdapterKind[])
        : provider.kind === "mock"
          ? (["mock"] as ProviderAdapterKind[])
          : (["open_ai_chat", "open_ai_responses"] as ProviderAdapterKind[]);
  const storedAdapter = canRestoreStored ? storedSelection?.adapter : null;
  const adapter =
    storedAdapter &&
    providerAdapters.has(storedAdapter) &&
    allowedAdapters.includes(storedAdapter)
      ? storedAdapter
      : null;
  return {
    connectionId: provider.id,
    modelId,
    ...(adapter ? { adapter } : {}),
    reasoningEffort: reconcileReasoningEffort(
      provider.kind,
      modelId,
      canRestoreStored
        ? storedSelection.reasoningEffort
        : (provider.reasoningEffort ?? null),
    ),
  };
}

export function parseLastActiveThreadIds(value: unknown): LastActiveThreadIds {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  const stored = value as Record<string, unknown>;
  return Object.fromEntries(
    (["work", "code", "flow"] as const).flatMap((mode) => {
      const threadId = stored[mode];
      return typeof threadId === "string" && threadId ? [[mode, threadId]] : [];
    }),
  );
}

export function readLastActiveThreadId(
  experienceMode: ExperienceMode,
): string | null {
  if (typeof window === "undefined") return null;
  try {
    const stored = parseLastActiveThreadIds(
      JSON.parse(
        window.localStorage.getItem(lastActiveThreadStorageKey) ?? "{}",
      ),
    );
    return stored[experienceMode] ?? null;
  } catch {
    return null;
  }
}

export function writeLastActiveThreadId(
  experienceMode: ExperienceMode,
  threadId: string,
): void {
  if (typeof window === "undefined" || !threadId) return;
  try {
    const stored = parseLastActiveThreadIds(
      JSON.parse(
        window.localStorage.getItem(lastActiveThreadStorageKey) ?? "{}",
      ),
    );
    window.localStorage.setItem(
      lastActiveThreadStorageKey,
      JSON.stringify({ ...stored, [experienceMode]: threadId }),
    );
  } catch {
    // The active task remains available for the current session if storage fails.
  }
}
