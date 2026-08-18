import {
  useEffect,
  useImperativeHandle,
  useLayoutEffect,
  useRef,
  useState,
  type ClipboardEvent as ReactClipboardEvent,
  type DragEvent as ReactDragEvent,
} from "react";
import { Paperclip } from "lucide-react";
import {
  composerContentText,
  composerExternalValueSyncAction,
  composerInputCommitPending,
  composerUndoEntries,
  composerVisibleText,
  normalizeComposerImageDeletionSnapshot,
  referencedImageIds,
  type ComposerHistorySnapshot,
} from "../../composerContent";
import type { ConversationMetrics } from "../../conversationMetrics";
import { hasFileDragPayload } from "../../fileDrop";
import { useDismissiblePopover } from "../../hooks/useDismissiblePopover";
import { ComposerMetrics } from "./ComposerMetrics";
import { ComposerContextBar } from "./ComposerContextBar";
import { ComposerSources } from "./ComposerSources";
import { ComposerToolbar } from "./ComposerToolbar";
import { ComposerWorkForm } from "./ComposerWorkForm";
import {
  ComposerImageContextMenu,
  type ComposerImageContextMenuState,
} from "./ComposerImageContextMenu";
import { ComposerSendButton } from "./ComposerSendButton";
import { ImageLightbox } from "./ImageLightbox";
import {
  cloneComposerHistorySnapshot,
  composerRangeAtPoint,
  composerRangesEqual,
  composerSnapshotAtSelection,
  createComposerImageReferenceNode,
  endOfComposerRange,
  imageFileFingerprint,
  isComposerImageFile,
  rangeBelongsToEditor,
  readComposerContent,
  readComposerContentParts,
  renderComposerSnapshot,
  type ComposerImageAttachment,
} from "./composerDom";
import { collaborationModePlaceholder } from "./composerModes";
import {
  type ComposerOpenMenu,
  type ExecutionPermissionMode,
  type NewTaskLaunchMode,
} from "./composerTypes";
import type { ComposerFileDropHandle } from "./useConversationFileDrop";
import type {
  AppSettings,
  CollaborationMode,
  ContextSourceFile,
  InlineImageAttachment,
  InlineMessageContentPart,
  Project,
  ProviderSettings,
  SkillDescriptor,
  ThreadModelSelection,
  WorkForm,
} from "../../types";

export {
  ConversationFileDropTarget,
  useConversationFileDrop,
} from "./useConversationFileDrop";
export type { ComposerFileDropHandle } from "./useConversationFileDrop";
export { newTaskLaunchModeLabel } from "./composerTypes";
export type {
  ExecutionPermissionMode,
  NewTaskLaunchMode,
} from "./composerTypes";

const MAX_COMPOSER_IMAGES = 10;
const MAX_COMPOSER_IMAGE_BYTES = 25 * 1024 * 1024;
const MAX_COMPOSER_HISTORY_ENTRIES = 200;
const COMPOSER_DRAFT_PUBLISH_DELAY_MS = 300;
export function Composer({
  autoFocus = false,
  fileDropHandleRef,
  fileDropScope = "composer",
  value,
  workForm,
  isSending,
  isRunning,
  isCancelling,
  queuedMessageCount = 0,
  metrics,
  providers,
  activeProviderId,
  modelSelection,
  permissionMode,
  collaborationMode,
  sandboxMode,
  contextSources,
  skills,
  selectedSkillIds,
  workspaceRoot,
  projectName,
  projects,
  launchMode,
  onChange,
  onSubmit,
  onCancel,
  onPickWorkspace,
  onSelectProject,
  onChangeLaunchMode,
  onChangePermissionMode,
  onChangeCollaborationMode,
  onChangeSandboxMode,
  onChangeModelSelection,
  onOpenSettings,
  onAddContextSources,
  onRemoveContextSource,
  onToggleSkill,
}: {
  autoFocus?: boolean;
  fileDropHandleRef?: { current: ComposerFileDropHandle | null };
  fileDropScope?: "composer" | "conversation";
  value: string;
  workForm?: WorkForm | null;
  isSending: boolean;
  isRunning: boolean;
  isCancelling: boolean;
  queuedMessageCount?: number;
  metrics?: ConversationMetrics | null;
  providers: ProviderSettings[];
  activeProviderId: string;
  modelSelection: ThreadModelSelection | null;
  permissionMode: AppSettings["permissionMode"];
  collaborationMode: CollaborationMode;
  sandboxMode: AppSettings["sandbox"]["sandboxMode"];
  contextSources: ContextSourceFile[];
  skills: SkillDescriptor[];
  selectedSkillIds: string[];
  workspaceRoot: string | null;
  projectName: string | null;
  projects: Project[];
  launchMode?: NewTaskLaunchMode;
  onChange(value: string): void;
  onSubmit(
    value: string,
    imageAttachments: InlineImageAttachment[],
    contentParts: InlineMessageContentPart[],
  ): Promise<boolean>;
  onCancel(): void;
  onPickWorkspace(): void;
  onSelectProject(projectId: string): void;
  onChangeLaunchMode?(mode: NewTaskLaunchMode): void;
  onChangePermissionMode(mode: ExecutionPermissionMode): void;
  onChangeCollaborationMode(mode: CollaborationMode): void;
  onChangeSandboxMode(mode: AppSettings["sandbox"]["sandboxMode"]): void;
  onChangeModelSelection(selection: ThreadModelSelection): void;
  onOpenSettings(): void;
  onAddContextSources(files?: File[]): Promise<void>;
  onRemoveContextSource(path: string): void;
  onToggleSkill(skillId: string): void;
}) {
  const [openMenu, setOpenMenu] = useState<ComposerOpenMenu>(null);
  const closeMenus = () => {
    setOpenMenu(null);
  };
  const toggleMenu = (menu: Exclude<ComposerOpenMenu, null>) => {
    setOpenMenu((current) => (current === menu ? null : menu));
  };
  const popoverRef = useDismissiblePopover(Boolean(openMenu), closeMenus);
  const draftRef = useRef(value);
  const [hasDraftText, setHasDraftText] = useState(Boolean(value.trim()));
  const [imageAttachments, setImageAttachments] = useState<
    ComposerImageAttachment[]
  >([]);
  const [hasInlineImageReferences, setHasInlineImageReferences] =
    useState(false);
  const [previewIndex, setPreviewIndex] = useState<number | null>(null);
  const [isDraggingFiles, setIsDraggingFiles] = useState(false);
  const [imageContextMenu, setImageContextMenu] =
    useState<ComposerImageContextMenuState | null>(null);
  const dragDepthRef = useRef(0);
  const preDragComposerRangeRef = useRef<Range | null>(null);
  const imageAttachmentsRef = useRef(imageAttachments);
  const editorRef = useRef<HTMLDivElement>(null);
  const contextFileInputRef = useRef<HTMLInputElement>(null);
  const savedComposerRangeRef = useRef<Range | null>(null);
  const contextMenuInsertionRangeRef = useRef<Range | null>(null);
  const contextMenuReferenceRef = useRef<HTMLElement | null>(null);
  const imageContextMenuRef = useRef<HTMLDivElement>(null);
  const composerUndoHistoryRef = useRef<ComposerHistorySnapshot[]>([]);
  const composerRedoHistoryRef = useRef<ComposerHistorySnapshot[]>([]);
  const currentComposerSnapshotRef = useRef<ComposerHistorySnapshot | null>(
    null,
  );
  const compositionStartSnapshotRef = useRef<ComposerHistorySnapshot | null>(
    null,
  );
  const pendingBeforeInputSnapshotRef = useRef<ComposerHistorySnapshot | null>(
    null,
  );
  const isComposingRef = useRef(false);
  const lastLocallyPublishedValueRef = useRef<string | null>(null);
  const deferredExternalValueRef = useRef<{ value: string } | null>(null);
  const pendingComposerPublishRef = useRef<string | null>(null);
  const composerPublishTimerRef = useRef<number | null>(null);
  const lastExternalComposerValueRef = useRef(value);

  useEffect(() => {
    imageAttachmentsRef.current = imageAttachments;
  }, [imageAttachments]);

  function cancelPendingComposerPublish() {
    if (composerPublishTimerRef.current !== null) {
      window.clearTimeout(composerPublishTimerRef.current);
      composerPublishTimerRef.current = null;
    }
    pendingComposerPublishRef.current = null;
  }

  function flushPendingComposerPublish() {
    const nextText = pendingComposerPublishRef.current;
    cancelPendingComposerPublish();
    if (nextText !== null) onChange(nextText);
  }

  function scheduleComposerPublish(text: string) {
    pendingComposerPublishRef.current = text;
    if (composerPublishTimerRef.current !== null) {
      window.clearTimeout(composerPublishTimerRef.current);
    }
    composerPublishTimerRef.current = window.setTimeout(() => {
      composerPublishTimerRef.current = null;
      const nextText = pendingComposerPublishRef.current;
      pendingComposerPublishRef.current = null;
      if (nextText === null) return;
      onChange(nextText);
    }, COMPOSER_DRAFT_PUBLISH_DELAY_MS);
  }

  useEffect(
    () => () => {
      flushPendingComposerPublish();
    },
    [],
  );

  useEffect(
    () => () => {
      imageAttachmentsRef.current.forEach((attachment) =>
        URL.revokeObjectURL(attachment.previewUrl),
      );
    },
    [],
  );

  function applyExternalComposerValue(nextValue: string) {
    const editor = editorRef.current;
    if (!editor) return;
    cancelPendingComposerPublish();
    deferredExternalValueRef.current = null;
    lastLocallyPublishedValueRef.current = null;
    const current = composerSnapshotAtSelection(editor);
    const currentText = composerVisibleText(current.parts);
    if (currentComposerSnapshotRef.current && currentText === nextValue) return;
    if (!currentComposerSnapshotRef.current && currentText === nextValue) {
      currentComposerSnapshotRef.current = current;
      return;
    }

    imageAttachmentsRef.current.forEach((attachment) =>
      URL.revokeObjectURL(attachment.previewUrl),
    );
    imageAttachmentsRef.current = [];
    setImageAttachments([]);
    setHasInlineImageReferences(false);
    setPreviewIndex(null);
    editor.textContent = nextValue;
    const next = readComposerContent(editor);
    currentComposerSnapshotRef.current = next;
    composerUndoHistoryRef.current = [];
    composerRedoHistoryRef.current = [];
    draftRef.current = nextValue;
    setHasDraftText(Boolean(nextValue.trim()));
  }

  useLayoutEffect(() => {
    const compositionPending =
      isComposingRef.current || Boolean(compositionStartSnapshotRef.current);
    const valueChangedSinceLastSync =
      value !== lastExternalComposerValueRef.current;
    lastExternalComposerValueRef.current = value;
    const action = composerExternalValueSyncAction({
      value,
      lastLocallyPublishedValue: lastLocallyPublishedValueRef.current,
      compositionPending,
      pendingLocalPublish: pendingComposerPublishRef.current !== null,
      lastExternalValue: valueChangedSinceLastSync
        ? undefined
        : lastExternalComposerValueRef.current,
    });
    if (action === "ignore") {
      deferredExternalValueRef.current = null;
      return;
    }
    if (action === "defer") {
      deferredExternalValueRef.current = { value };
      return;
    }
    applyExternalComposerValue(value);
  }, [value]);

  useEffect(() => {
    const rememberSelection = () => {
      const editor = editorRef.current;
      const selection = window.getSelection();
      if (!editor || !selection || selection.rangeCount === 0) return;
      const range = selection.getRangeAt(0);
      if (rangeBelongsToEditor(editor, range)) {
        if (composerRangesEqual(savedComposerRangeRef.current, range)) return;
        savedComposerRangeRef.current = range.cloneRange();
        if (!isComposingRef.current && currentComposerSnapshotRef.current) {
          currentComposerSnapshotRef.current = {
            ...currentComposerSnapshotRef.current,
            caretOffset: readComposerContent(editor, {
              node: range.startContainer,
              offset: range.startOffset,
            }).caretOffset,
          };
        }
      }
    };
    document.addEventListener("selectionchange", rememberSelection);
    return () =>
      document.removeEventListener("selectionchange", rememberSelection);
  }, []);

  useEffect(() => {
    if (!imageContextMenu) return;
    const close = () => setImageContextMenu(null);
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") close();
    };
    window.addEventListener("pointerdown", close);
    window.addEventListener("blur", close);
    window.addEventListener("keydown", closeOnEscape);
    return () => {
      window.removeEventListener("pointerdown", close);
      window.removeEventListener("blur", close);
      window.removeEventListener("keydown", closeOnEscape);
    };
  }, [imageContextMenu]);

  useLayoutEffect(() => {
    const menu = imageContextMenuRef.current;
    if (!menu || !imageContextMenu) return;
    const bounds = menu.getBoundingClientRect();
    const nextX = Math.max(
      0,
      imageContextMenu.x - Math.max(0, bounds.right - window.innerWidth),
    );
    const nextY = Math.max(
      0,
      imageContextMenu.y - Math.max(0, bounds.bottom - window.innerHeight),
    );
    if (nextX !== imageContextMenu.x || nextY !== imageContextMenu.y) {
      setImageContextMenu((current) =>
        current ? { ...current, x: nextX, y: nextY } : null,
      );
    }
  }, [imageContextMenu]);

  function publishComposerSnapshot(snapshot: ComposerHistorySnapshot) {
    const usedIds = referencedImageIds(snapshot.parts);
    const currentAttachments = imageAttachmentsRef.current;
    const text = composerVisibleText(snapshot.parts);
    setHasInlineImageReferences(usedIds.size > 0);
    setPreviewIndex((current) => {
      const attachment =
        current === null ? undefined : currentAttachments[current];
      return attachment && !usedIds.has(attachment.id) ? null : current;
    });
    draftRef.current = text;
    setHasDraftText(Boolean(text.trim()));
    lastLocallyPublishedValueRef.current = text;
    scheduleComposerPublish(text);
  }

  function commitComposerMutation(
    splitInsertedContent: boolean,
    requestedBefore?: ComposerHistorySnapshot | null,
    normalizeImageDeletionArtifacts = false,
  ) {
    const editor = editorRef.current;
    if (!editor) return;
    if (
      !editor.textContent &&
      !editor.querySelector("[data-composer-image-id]")
    ) {
      editor.replaceChildren();
    }
    let after = composerSnapshotAtSelection(editor);
    const before =
      requestedBefore ?? currentComposerSnapshotRef.current ?? after;
    if (normalizeImageDeletionArtifacts) {
      const normalizedAfter = normalizeComposerImageDeletionSnapshot(
        before,
        after,
      );
      if (normalizedAfter !== after) {
        const range = renderComposerSnapshot(
          editor,
          normalizedAfter,
          imageAttachmentsRef.current,
        );
        const selection = window.getSelection();
        selection?.removeAllRanges();
        selection?.addRange(range);
        savedComposerRangeRef.current = range.cloneRange();
        after = normalizedAfter;
      }
    }
    const entries = composerUndoEntries(before, after, splitInsertedContent);
    if (entries.length > 0) {
      composerUndoHistoryRef.current.push(...entries);
      if (
        composerUndoHistoryRef.current.length > MAX_COMPOSER_HISTORY_ENTRIES
      ) {
        composerUndoHistoryRef.current.splice(
          0,
          composerUndoHistoryRef.current.length - MAX_COMPOSER_HISTORY_ENTRIES,
        );
      }
      composerRedoHistoryRef.current = [];
    }
    currentComposerSnapshotRef.current = cloneComposerHistorySnapshot(after);
    const selection = window.getSelection();
    if (selection && selection.rangeCount > 0) {
      const range = selection.getRangeAt(0);
      if (rangeBelongsToEditor(editor, range)) {
        savedComposerRangeRef.current = range.cloneRange();
      }
    }
    publishComposerSnapshot(after);
  }

  function restoreComposerHistorySnapshot(snapshot: ComposerHistorySnapshot) {
    const editor = editorRef.current;
    if (!editor) return;
    const restored = cloneComposerHistorySnapshot(snapshot);
    const range = renderComposerSnapshot(
      editor,
      restored,
      imageAttachmentsRef.current,
    );
    const selection = window.getSelection();
    selection?.removeAllRanges();
    selection?.addRange(range);
    savedComposerRangeRef.current = range.cloneRange();
    currentComposerSnapshotRef.current = restored;
    compositionStartSnapshotRef.current = null;
    pendingBeforeInputSnapshotRef.current = null;
    deferredExternalValueRef.current = null;
    setImageContextMenu(null);
    editor.focus();
    publishComposerSnapshot(restored);
  }

  function undoComposerMutation() {
    const target = composerUndoHistoryRef.current.pop();
    const editor = editorRef.current;
    if (!target || !editor) return;
    composerRedoHistoryRef.current.push(composerSnapshotAtSelection(editor));
    restoreComposerHistorySnapshot(target);
  }

  function redoComposerMutation() {
    const target = composerRedoHistoryRef.current.pop();
    const editor = editorRef.current;
    if (!target || !editor) return;
    composerUndoHistoryRef.current.push(composerSnapshotAtSelection(editor));
    restoreComposerHistorySnapshot(target);
  }

  function insertImageReference(
    attachment: ComposerImageAttachment,
    requestedRange: Range | null = savedComposerRangeRef.current,
  ) {
    const editor = editorRef.current;
    if (!editor) return;
    const range = rangeBelongsToEditor(editor, requestedRange)
      ? requestedRange!.cloneRange()
      : endOfComposerRange(editor);
    const node = createComposerImageReferenceNode(attachment);
    range.deleteContents();
    range.insertNode(node);
    range.setStartAfter(node);
    range.collapse(true);
    const selection = window.getSelection();
    selection?.removeAllRanges();
    selection?.addRange(range);
    savedComposerRangeRef.current = range.cloneRange();
    editor.focus();
  }

  async function addImageFiles(
    files: File[],
    requestedRange: Range | null = savedComposerRangeRef.current,
  ) {
    const acceptedFiles = files
      .filter(isComposerImageFile)
      .filter((file) => file.size <= MAX_COMPOSER_IMAGE_BYTES);
    if (acceptedFiles.length === 0) return;

    const next: ComposerImageAttachment[] = [];
    const references: ComposerImageAttachment[] = [];
    for (const file of acceptedFiles) {
      const { data, fingerprint } = await imageFileFingerprint(file);
      const existing = [...imageAttachmentsRef.current, ...next].find(
        (attachment) => attachment.fingerprint === fingerprint,
      );
      if (existing) {
        references.push(existing);
        continue;
      }
      if (
        imageAttachmentsRef.current.length + next.length >=
        MAX_COMPOSER_IMAGES
      ) {
        continue;
      }
      const attachment = {
        id: crypto.randomUUID(),
        name: file.name || `pasted-image-${next.length + 1}.png`,
        contentType: file.type || "image/png",
        data,
        previewUrl: URL.createObjectURL(file),
        fingerprint,
      };
      next.push(attachment);
      references.push(attachment);
    }
    if (references.length === 0) return;
    const combined = [...imageAttachmentsRef.current, ...next];
    imageAttachmentsRef.current = combined;
    setImageAttachments(combined);

    let range = requestedRange?.cloneRange() ?? null;
    const editor = editorRef.current;
    const before =
      editor && rangeBelongsToEditor(editor, range)
        ? readComposerContent(editor, {
            node: range!.startContainer,
            offset: range!.startOffset,
          })
        : editor
          ? composerSnapshotAtSelection(editor)
          : null;
    for (const attachment of references) {
      insertImageReference(attachment, range);
      range = savedComposerRangeRef.current?.cloneRange() ?? null;
    }
    commitComposerMutation(true, before);
  }

  function removeImageReference(reference: HTMLElement | null) {
    if (!reference?.isConnected) return;
    const editor = editorRef.current;
    const before = editor ? composerSnapshotAtSelection(editor) : null;
    reference.remove();
    commitComposerMutation(false, before, true);
  }

  const submitDraft = async () => {
    if (isSending) return;
    flushPendingComposerPublish();
    const editor = editorRef.current;
    const parts = editor ? readComposerContentParts(editor) : [];
    const usedIds = referencedImageIds(parts);
    const currentAttachments = imageAttachmentsRef.current;
    const submittedAttachments = currentAttachments
      .filter((attachment) => usedIds.has(attachment.id))
      .map(
        ({
          previewUrl: _previewUrl,
          fingerprint: _fingerprint,
          ...attachment
        }) => attachment,
      );
    const submittedValue =
      submittedAttachments.length > 0
        ? composerContentText(parts, submittedAttachments)
        : draftRef.current;
    const accepted = await onSubmit(
      submittedValue,
      submittedAttachments,
      submittedAttachments.length > 0 ? parts : [],
    );
    if (!accepted) return;
    currentAttachments.forEach((attachment) =>
      URL.revokeObjectURL(attachment.previewUrl),
    );
    setImageAttachments([]);
    imageAttachmentsRef.current = [];
    setHasInlineImageReferences(false);
    setPreviewIndex(null);
    if (editor) editor.replaceChildren();
    currentComposerSnapshotRef.current = {
      parts: [],
      caretOffset: 0,
    };
    composerUndoHistoryRef.current = [];
    composerRedoHistoryRef.current = [];
    compositionStartSnapshotRef.current = null;
    pendingBeforeInputSnapshotRef.current = null;
    deferredExternalValueRef.current = null;
    draftRef.current = "";
    setHasDraftText(false);
    lastLocallyPublishedValueRef.current = "";
    onChange("");
  };

  async function handlePaste(event: ReactClipboardEvent<HTMLDivElement>) {
    const items = Array.from(event.clipboardData.items).filter(
      (item) => item.kind === "file" && item.type.startsWith("image/"),
    );
    if (items.length === 0) return;

    event.preventDefault();
    const range = savedComposerRangeRef.current?.cloneRange() ?? null;
    const files = items
      .map((item) => item.getAsFile())
      .filter((file): file is File => Boolean(file));
    await addImageFiles(files, range);
  }

  function addSelectedFiles(
    files: File[],
    requestedRange: Range | null = savedComposerRangeRef.current,
  ) {
    const images = files.filter(isComposerImageFile);
    const otherFiles = files.filter((file) => !isComposerImageFile(file));
    if (images.length > 0) {
      void addImageFiles(images, requestedRange?.cloneRange() ?? null);
    }
    if (otherFiles.length > 0) void onAddContextSources(otherFiles);
  }

  useImperativeHandle(fileDropHandleRef, () => ({
    addFiles(files) {
      addSelectedFiles(files);
    },
  }));

  function handleDragEnter(event: ReactDragEvent<HTMLDivElement>) {
    if (!hasFileDragPayload(event.dataTransfer.types)) return;
    event.preventDefault();
    if (dragDepthRef.current === 0) {
      const editor = editorRef.current;
      const savedRange = savedComposerRangeRef.current;
      preDragComposerRangeRef.current =
        editor && rangeBelongsToEditor(editor, savedRange)
          ? savedRange!.cloneRange()
          : null;
    }
    dragDepthRef.current += 1;
    setIsDraggingFiles(true);
  }

  function handleDragOver(event: ReactDragEvent<HTMLDivElement>) {
    if (!hasFileDragPayload(event.dataTransfer.types)) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "copy";
  }

  function handleDragLeave() {
    dragDepthRef.current = Math.max(0, dragDepthRef.current - 1);
    if (dragDepthRef.current === 0) {
      preDragComposerRangeRef.current = null;
      setIsDraggingFiles(false);
    }
  }

  function handleDrop(event: ReactDragEvent<HTMLDivElement>) {
    if (!hasFileDragPayload(event.dataTransfer.types)) return;
    event.preventDefault();
    dragDepthRef.current = 0;
    setIsDraggingFiles(false);
    const files = Array.from(event.dataTransfer.files);
    const range =
      preDragComposerRangeRef.current?.cloneRange() ??
      composerRangeAtPoint(event.clientX, event.clientY);
    preDragComposerRangeRef.current = null;
    addSelectedFiles(files, range);
  }

  const hasSendableContent = Boolean(
    hasDraftText ||
    hasInlineImageReferences ||
    contextSources.length > 0 ||
    selectedSkillIds.length > 0,
  );
  return (
    <div className={`composer-shell${metrics ? " has-metrics" : ""}`}>
      {workForm ? <ComposerWorkForm form={workForm} /> : null}
      <div
        className={`composer ${workspaceRoot || projectName ? "has-context" : ""} ${contextSources.length || selectedSkillIds.length ? "has-sources" : ""} ${isDraggingFiles ? "is-dragging-files" : ""}`}
        ref={popoverRef}
        onDragEnter={fileDropScope === "composer" ? handleDragEnter : undefined}
        onDragOver={fileDropScope === "composer" ? handleDragOver : undefined}
        onDragLeave={fileDropScope === "composer" ? handleDragLeave : undefined}
        onDrop={fileDropScope === "composer" ? handleDrop : undefined}
      >
        <ComposerContextBar
          openMenu={openMenu}
          workspaceRoot={workspaceRoot}
          projectName={projectName}
          projects={projects}
          launchMode={launchMode}
          sandboxMode={sandboxMode}
          onToggleMenu={toggleMenu}
          onCloseMenu={closeMenus}
          onPickWorkspace={onPickWorkspace}
          onSelectProject={onSelectProject}
          onChangeLaunchMode={onChangeLaunchMode}
          onChangeSandboxMode={onChangeSandboxMode}
        />
        <ComposerSources
          contextSources={contextSources}
          skills={skills}
          selectedSkillIds={selectedSkillIds}
          onRemoveContextSource={onRemoveContextSource}
          onToggleSkill={onToggleSkill}
        />
        {isDraggingFiles ? (
          <div className="composer-drop-target" aria-hidden="true">
            <Paperclip size={20} />
            <span>释放以添加文件</span>
          </div>
        ) : null}
        <div
          ref={editorRef}
          autoFocus={autoFocus}
          className="composer-rich-input"
          contentEditable
          suppressContentEditableWarning
          role="textbox"
          aria-label="消息"
          aria-multiline="true"
          data-placeholder={collaborationModePlaceholder(collaborationMode)}
          onFocus={closeMenus}
          onPointerDown={(event) => {
            closeMenus();
            if (
              event.button === 2 &&
              (event.target as Element).closest("[data-composer-image-id]")
            ) {
              contextMenuInsertionRangeRef.current =
                savedComposerRangeRef.current?.cloneRange() ?? null;
            }
          }}
          onPaste={(event) => void handlePaste(event)}
          onBeforeInput={(event) => {
            const inputType = (event.nativeEvent as InputEvent).inputType;
            if (inputType === "historyUndo") {
              event.preventDefault();
              undoComposerMutation();
            } else if (inputType === "historyRedo") {
              event.preventDefault();
              redoComposerMutation();
            } else if (
              !isComposingRef.current &&
              !compositionStartSnapshotRef.current
            ) {
              const editor = editorRef.current;
              pendingBeforeInputSnapshotRef.current =
                currentComposerSnapshotRef.current
                  ? cloneComposerHistorySnapshot(
                      currentComposerSnapshotRef.current,
                    )
                  : editor
                    ? composerSnapshotAtSelection(editor)
                    : null;
            }
          }}
          onCompositionStart={() => {
            const editor = editorRef.current;
            isComposingRef.current = true;
            pendingBeforeInputSnapshotRef.current = null;
            compositionStartSnapshotRef.current =
              currentComposerSnapshotRef.current
                ? cloneComposerHistorySnapshot(
                    currentComposerSnapshotRef.current,
                  )
                : editor
                  ? composerSnapshotAtSelection(editor)
                  : null;
          }}
          onCompositionEnd={() => {
            isComposingRef.current = false;
            queueMicrotask(() => {
              const before = compositionStartSnapshotRef.current;
              compositionStartSnapshotRef.current = null;
              pendingBeforeInputSnapshotRef.current = null;
              const deferredExternalValue = deferredExternalValueRef.current;
              if (deferredExternalValue) {
                applyExternalComposerValue(deferredExternalValue.value);
                return;
              }
              commitComposerMutation(true, before);
            });
          }}
          onInput={(event) => {
            const nativeEvent = event.nativeEvent as InputEvent;
            if (
              composerInputCommitPending({
                isComposing: isComposingRef.current,
                compositionSnapshotPending: Boolean(
                  compositionStartSnapshotRef.current,
                ),
                nativeIsComposing: nativeEvent.isComposing,
              })
            )
              return;
            const before = pendingBeforeInputSnapshotRef.current;
            pendingBeforeInputSnapshotRef.current = null;
            commitComposerMutation(
              nativeEvent.inputType === "insertText" ||
                nativeEvent.inputType === "insertCompositionText" ||
                nativeEvent.inputType === "insertFromComposition",
              before,
              nativeEvent.inputType.startsWith("delete"),
            );
          }}
          onClick={(event) => {
            const target = (event.target as Element).closest<HTMLElement>(
              ".composer-inline-image-button",
            );
            const imageId = target?.dataset.composerImageId;
            if (!imageId) return;
            const index = imageAttachmentsRef.current.findIndex(
              (attachment) => attachment.id === imageId,
            );
            if (index >= 0) setPreviewIndex(index);
          }}
          onContextMenu={(event) => {
            const reference = (event.target as Element).closest<HTMLElement>(
              ".composer-inline-image-reference",
            );
            const imageId = reference?.dataset.composerImageId;
            if (!imageId) return;
            event.preventDefault();
            contextMenuReferenceRef.current = reference;
            setImageContextMenu({
              imageId,
              x: event.clientX,
              y: event.clientY,
            });
          }}
          onKeyDown={(event) => {
            if (event.nativeEvent.isComposing) return;
            const primaryModifier = event.ctrlKey || event.metaKey;
            if (
              primaryModifier &&
              !event.altKey &&
              event.key.toLocaleLowerCase() === "z"
            ) {
              event.preventDefault();
              if (event.shiftKey) redoComposerMutation();
              else undoComposerMutation();
              return;
            }
            if (
              event.ctrlKey &&
              !event.altKey &&
              !event.shiftKey &&
              event.key.toLocaleLowerCase() === "y"
            ) {
              event.preventDefault();
              redoComposerMutation();
              return;
            }
            if (
              (event.target as Element).closest(".composer-inline-image-button")
            ) {
              return;
            }
            if (
              event.key === "Enter" &&
              !event.altKey &&
              !event.nativeEvent.isComposing &&
              !event.repeat
            ) {
              event.preventDefault();
              submitDraft();
            }
          }}
        />
        <input
          ref={contextFileInputRef}
          hidden
          type="file"
          multiple
          onChange={(event) => {
            const files = Array.from(event.target.files ?? []);
            event.target.value = "";
            addSelectedFiles(
              files,
              savedComposerRangeRef.current?.cloneRange() ?? null,
            );
          }}
        />
        <ComposerToolbar
          openMenu={openMenu}
          queuedMessageCount={queuedMessageCount}
          providers={providers}
          activeProviderId={activeProviderId}
          modelSelection={modelSelection}
          permissionMode={permissionMode}
          collaborationMode={collaborationMode}
          skills={skills}
          selectedSkillIds={selectedSkillIds}
          isRunning={isRunning}
          isSending={isSending}
          onToggleMenu={toggleMenu}
          onCloseMenu={closeMenus}
          onPickFiles={() => contextFileInputRef.current?.click()}
          onChangePermissionMode={onChangePermissionMode}
          onChangeCollaborationMode={onChangeCollaborationMode}
          onChangeModelSelection={onChangeModelSelection}
          onOpenSettings={onOpenSettings}
          onToggleSkill={onToggleSkill}
        />
        <ComposerSendButton
          hasSendableContent={hasSendableContent}
          isSending={isSending}
          isRunning={isRunning}
          isCancelling={isCancelling}
          onSubmit={submitDraft}
          onCancel={onCancel}
        />
      </div>
      {metrics ? <ComposerMetrics metrics={metrics} /> : null}
      {previewIndex !== null && imageAttachments[previewIndex] ? (
        <ImageLightbox
          attachments={imageAttachments}
          activeIndex={previewIndex}
          onChangeIndex={setPreviewIndex}
          onClose={() => setPreviewIndex(null)}
        />
      ) : null}
      {imageContextMenu ? (
        <ComposerImageContextMenu
          state={imageContextMenu}
          menuRef={imageContextMenuRef}
          onQuote={() => {
            const attachment = imageAttachmentsRef.current.find(
              (item) => item.id === imageContextMenu.imageId,
            );
            if (attachment) {
              const editor = editorRef.current;
              const requestedRange = contextMenuInsertionRangeRef.current;
              const before =
                editor && rangeBelongsToEditor(editor, requestedRange)
                  ? readComposerContent(editor, {
                      node: requestedRange!.startContainer,
                      offset: requestedRange!.startOffset,
                    })
                  : editor
                    ? composerSnapshotAtSelection(editor)
                    : null;
              insertImageReference(attachment, requestedRange);
              commitComposerMutation(true, before);
            }
            contextMenuInsertionRangeRef.current = null;
            setImageContextMenu(null);
          }}
          onRemove={() => {
            removeImageReference(contextMenuReferenceRef.current);
            contextMenuInsertionRangeRef.current = null;
            contextMenuReferenceRef.current = null;
            setImageContextMenu(null);
          }}
        />
      ) : null}
    </div>
  );
}
