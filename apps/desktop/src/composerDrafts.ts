import type { ContextSourceFile, ExperienceMode } from "./types";

export type ComposerDraft = {
  text: string;
  contextSources: ContextSourceFile[];
  selectedSkillIds: string[];
  updatedAt: number;
};

export type ComposerDrafts = Readonly<Record<string, ComposerDraft>>;

const storageKey = "opentopia.composer-drafts.v1";
const maxStoredDrafts = 200;
const storageWriteDelayMs = 150;
const listeners = new Set<() => void>();
let snapshot: ComposerDrafts | null = null;
let storageWriteTimer: number | null = null;
let flushRegistered = false;

export function threadComposerDraftKey(threadId: string): string {
  return `thread:${threadId}`;
}

export function newTaskComposerDraftKey(
  experienceMode: ExperienceMode,
  projectId: string | null,
): string {
  return `new:${experienceMode}:${projectId ?? "unassigned"}`;
}

export function emptyComposerDraft(): ComposerDraft {
  return {
    text: "",
    contextSources: [],
    selectedSkillIds: [],
    updatedAt: 0,
  };
}

function parseContextSource(value: unknown): ContextSourceFile | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const source = value as Record<string, unknown>;
  if (
    typeof source.path !== "string" ||
    !source.path ||
    typeof source.name !== "string" ||
    typeof source.extension !== "string" ||
    (source.kind !== "text" &&
      source.kind !== "image" &&
      source.kind !== "document") ||
    typeof source.bytes !== "number" ||
    !Number.isFinite(source.bytes) ||
    source.bytes < 0
  ) {
    return null;
  }
  return {
    path: source.path,
    name: source.name,
    extension: source.extension,
    kind: source.kind,
    bytes: source.bytes,
  };
}

export function parseComposerDrafts(value: unknown): ComposerDrafts {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  const entries: Array<[string, ComposerDraft]> = [];
  for (const [draftKey, rawDraft] of Object.entries(value)) {
    if (
      !draftKey ||
      !rawDraft ||
      typeof rawDraft !== "object" ||
      Array.isArray(rawDraft)
    ) {
      continue;
    }
    const draft = rawDraft as Record<string, unknown>;
    if (typeof draft.text !== "string") continue;
    const contextSources = Array.isArray(draft.contextSources)
      ? draft.contextSources
          .map(parseContextSource)
          .filter((source): source is ContextSourceFile => Boolean(source))
          .slice(0, 20)
      : [];
    const selectedSkillIds = Array.isArray(draft.selectedSkillIds)
      ? Array.from(
          new Set(
            draft.selectedSkillIds.filter(
              (skillId): skillId is string =>
                typeof skillId === "string" && skillId.length > 0,
            ),
          ),
        ).slice(0, 5)
      : [];
    const updatedAt =
      typeof draft.updatedAt === "number" &&
      Number.isFinite(draft.updatedAt) &&
      draft.updatedAt >= 0
        ? draft.updatedAt
        : 0;
    if (
      !draft.text &&
      contextSources.length === 0 &&
      selectedSkillIds.length === 0
    ) {
      continue;
    }
    entries.push([
      draftKey,
      {
        text: draft.text,
        contextSources,
        selectedSkillIds,
        updatedAt,
      },
    ]);
  }
  return Object.fromEntries(
    entries
      .sort((left, right) => right[1].updatedAt - left[1].updatedAt)
      .slice(0, maxStoredDrafts),
  );
}

function readStoredDrafts(): ComposerDrafts {
  if (typeof window === "undefined") return {};
  try {
    return parseComposerDrafts(
      JSON.parse(window.localStorage.getItem(storageKey) ?? "{}"),
    );
  } catch {
    return {};
  }
}

function persistDrafts(drafts: ComposerDrafts): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(storageKey, JSON.stringify(drafts));
  } catch {
    // Drafts remain available in memory if browser storage is unavailable.
  }
}

function flushDrafts(): void {
  if (storageWriteTimer !== null) {
    window.clearTimeout(storageWriteTimer);
    storageWriteTimer = null;
  }
  if (snapshot) persistDrafts(snapshot);
}

function scheduleDraftPersistence(): void {
  if (typeof window === "undefined") return;
  if (!flushRegistered) {
    window.addEventListener("beforeunload", flushDrafts);
    flushRegistered = true;
  }
  if (storageWriteTimer !== null) window.clearTimeout(storageWriteTimer);
  storageWriteTimer = window.setTimeout(flushDrafts, storageWriteDelayMs);
}

export function getComposerDraftsSnapshot(): ComposerDrafts {
  snapshot ??= readStoredDrafts();
  return snapshot;
}

export function subscribeComposerDrafts(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function isEmptyDraft(draft: ComposerDraft): boolean {
  return (
    !draft.text &&
    draft.contextSources.length === 0 &&
    draft.selectedSkillIds.length === 0
  );
}

function pruneOldDrafts(drafts: Record<string, ComposerDraft>): ComposerDrafts {
  const entries = Object.entries(drafts);
  if (entries.length <= maxStoredDrafts) return drafts;
  return Object.fromEntries(
    entries
      .sort((left, right) => right[1].updatedAt - left[1].updatedAt)
      .slice(0, maxStoredDrafts),
  );
}

export function updateComposerDraft(
  draftKey: string,
  update: (current: ComposerDraft) => ComposerDraft,
): void {
  const currentDraft =
    getComposerDraftsSnapshot()[draftKey] ?? emptyComposerDraft();
  const updated = update(currentDraft);
  const nextDraft = {
    ...updated,
    contextSources: updated.contextSources.slice(0, 20),
    selectedSkillIds: Array.from(new Set(updated.selectedSkillIds)).slice(0, 5),
    updatedAt: Date.now(),
  };
  const next = { ...getComposerDraftsSnapshot() };
  if (isEmptyDraft(nextDraft)) delete next[draftKey];
  else next[draftKey] = nextDraft;
  snapshot = pruneOldDrafts(next);
  scheduleDraftPersistence();
  listeners.forEach((listener) => listener());
}
