import {
  useCallback,
  useSyncExternalStore,
  type Dispatch,
  type SetStateAction,
} from "react";
import {
  composerDraftContentPartsFromLegacyText,
  emptyComposerDraft,
  getComposerDraftsSnapshot,
  normalizeComposerDraftContentParts,
  subscribeComposerDrafts,
  updateComposerDraft,
} from "./composerDrafts";
import type { ComposerDraftContentPart } from "./composerDrafts";
import type { ContextSourceFile, InlineMessageContentPart } from "./types";

export function useComposerDraft(draftKey: string): {
  text: string;
  contentParts: ComposerDraftContentPart[];
  contextSources: ContextSourceFile[];
  selectedSkillIds: string[];
  setText: Dispatch<SetStateAction<string>>;
  setContent(text: string, contentParts: InlineMessageContentPart[]): void;
  setContextSources: Dispatch<SetStateAction<ContextSourceFile[]>>;
  setSelectedSkillIds: Dispatch<SetStateAction<string[]>>;
} {
  const drafts = useSyncExternalStore(
    subscribeComposerDrafts,
    getComposerDraftsSnapshot,
    getComposerDraftsSnapshot,
  );
  const draft = drafts[draftKey] ?? emptyComposerDraft();
  const contentParts =
    draft.contentParts ??
    composerDraftContentPartsFromLegacyText(draft.text, draft.contextSources);

  const setText = useCallback<Dispatch<SetStateAction<string>>>(
    (update) => {
      updateComposerDraft(draftKey, (current) => ({
        ...current,
        contentParts: [
          {
            type: "text",
            text: typeof update === "function" ? update(current.text) : update,
          },
        ],
      }));
    },
    [draftKey],
  );
  const setContent = useCallback(
    (_text: string, contentParts: InlineMessageContentPart[]) => {
      updateComposerDraft(draftKey, (current) => ({
        ...current,
        contentParts: normalizeComposerDraftContentParts(
          contentParts,
          current.contextSources,
        ),
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
    contentParts,
    contextSources: draft.contextSources,
    selectedSkillIds: draft.selectedSkillIds,
    setText,
    setContent,
    setContextSources,
    setSelectedSkillIds,
  };
}
