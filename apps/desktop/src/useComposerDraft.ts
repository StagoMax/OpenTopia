import {
  useCallback,
  useSyncExternalStore,
  type Dispatch,
  type SetStateAction,
} from "react";
import {
  emptyComposerDraft,
  getComposerDraftsSnapshot,
  subscribeComposerDrafts,
  updateComposerDraft,
} from "./composerDrafts";
import type { ContextSourceFile } from "./types";

export function useComposerDraft(draftKey: string): {
  text: string;
  contextSources: ContextSourceFile[];
  selectedSkillIds: string[];
  setText: Dispatch<SetStateAction<string>>;
  setContextSources: Dispatch<SetStateAction<ContextSourceFile[]>>;
  setSelectedSkillIds: Dispatch<SetStateAction<string[]>>;
} {
  const drafts = useSyncExternalStore(
    subscribeComposerDrafts,
    getComposerDraftsSnapshot,
    getComposerDraftsSnapshot,
  );
  const draft = drafts[draftKey] ?? emptyComposerDraft();

  const setText = useCallback<Dispatch<SetStateAction<string>>>(
    (update) => {
      updateComposerDraft(draftKey, (current) => ({
        ...current,
        text: typeof update === "function" ? update(current.text) : update,
      }));
    },
    [draftKey],
  );
  const setContextSources = useCallback<
    Dispatch<SetStateAction<ContextSourceFile[]>>
  >(
    (update) => {
      updateComposerDraft(draftKey, (current) => ({
        ...current,
        contextSources:
          typeof update === "function"
            ? update(current.contextSources)
            : update,
      }));
    },
    [draftKey],
  );
  const setSelectedSkillIds = useCallback<Dispatch<SetStateAction<string[]>>>(
    (update) => {
      updateComposerDraft(draftKey, (current) => ({
        ...current,
        selectedSkillIds:
          typeof update === "function"
            ? update(current.selectedSkillIds)
            : update,
      }));
    },
    [draftKey],
  );

  return {
    text: draft.text,
    contextSources: draft.contextSources,
    selectedSkillIds: draft.selectedSkillIds,
    setText,
    setContextSources,
    setSelectedSkillIds,
  };
}
