import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AlertCircle } from "lucide-react";
import { ApiClient } from "../../api/client";
import {
  ApprovalDialog,
  type ApprovalRequest,
} from "../../components/ApprovalDialog";
import { PlanChoiceCard } from "../../components/PlanChoiceCard";
import type { ImagePreviewSource } from "../../components/PreviewHost";
import {
  Composer,
  ConversationFileDropTarget,
  useConversationFileDrop,
  type ComposerFileDropHandle,
  type ExecutionPermissionMode,
} from "../composer/Composer";
import {
  ConversationLoadErrorState,
  ConversationLoadingState,
} from "../conversation/ConversationHeader";
import { MessageList } from "../conversation/MessageList";
import { conversationMetrics as deriveConversationMetrics } from "../../conversationMetrics";
import { threadComposerDraftKey } from "../../composerDrafts";
import { resolveComposerWorkForm } from "../../conversationWorkForm";
import { ConversationSessionRegistry } from "../../conversationSessionController";
import { errorMessage } from "../../errorMessage";
import type { SendShortcut } from "../../editorPreferences";
import { getDroppedContextFiles, selectContextFiles } from "../../platform";
import { friendlyProviderError } from "../../providerErrors";
import { resolveThreadModelContextWindow } from "../../modelCapabilities";
import {
  resolveThreadActivityStatus,
  type ThreadActivityStatus,
} from "../../threadActivityStatus";
import {
  threadTitleFromPrompt,
  threadTitleNeedsSummary,
} from "../../threadTitle";
import type { ToolTabKind } from "../../toolTabs";
import type {
  AgentEvent,
  AppSettings,
  CollaborationMode,
  InlineImageAttachment,
  InlineMessageContentPart,
  PreviewTarget,
  Project,
  SkillDescriptor,
  Thread,
  ThreadModelSelection,
  UserInputResponse,
} from "../../types";
import { useComposerDraft } from "../../useComposerDraft";
import { useConversationSession } from "../../useConversationSession";
import { workspaceRootKey } from "../../workspaceRootKey";

export function SideTaskConversation({
  client,
  conversationRegistry,
  thread,
  settings,
  projects,
  skills,
  initialCollaborationMode,
  sendShortcut,
  showContextWindowUsage,
  onThreadUpdated,
  onSetThreadActivity,
  onMarkThreadActivityRead,
  onChangePermissionMode,
  onChangeSandboxMode,
  onOpenSettings,
  onOpenArtifact,
  onOpenImagePreview,
  onOpenPreview,
  onOpenMarkdownLink,
  onOpenToolTab,
  onOpenFileReview,
}: {
  client: ApiClient | null;
  conversationRegistry: ConversationSessionRegistry | null;
  thread: Thread | null;
  settings: AppSettings | null;
  projects: Project[];
  skills: SkillDescriptor[];
  initialCollaborationMode: CollaborationMode;
  sendShortcut: SendShortcut;
  showContextWindowUsage: boolean;
  onThreadUpdated(thread: Thread): void;
  onSetThreadActivity(
    threadId: string,
    status: ThreadActivityStatus | null,
  ): void;
  onMarkThreadActivityRead(threadId: string): void;
  onChangePermissionMode(mode: ExecutionPermissionMode): void;
  onChangeSandboxMode(mode: AppSettings["sandbox"]["sandboxMode"]): void;
  onOpenSettings(): void;
  onOpenArtifact(threadId: string, artifactId: string): void;
  onOpenImagePreview(
    threadId: string,
    sourceId: string,
    image: ImagePreviewSource,
  ): void;
  onOpenPreview(threadId: string, target: PreviewTarget, title: string): void;
  onOpenMarkdownLink(href: string, baseWorkspacePath?: string | null): void;
  onOpenToolTab(kind: ToolTabKind): void;
  onOpenFileReview(path: string): void;
}) {
  const threadId = thread?.id ?? null;
  const {
    text: composer,
    contextSources,
    selectedSkillIds,
    setText: setComposer,
    setContextSources,
    setSelectedSkillIds,
  } = useComposerDraft(threadComposerDraftKey(threadId ?? "unavailable"));
  const [collaborationMode, setCollaborationMode] = useState(
    initialCollaborationMode,
  );
  const [modelSelection, setModelSelection] =
    useState<ThreadModelSelection | null>(thread?.modelSelection ?? null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [undoingTurnId, setUndoingTurnId] = useState<string | null>(null);
  const composerFileDropHandle = useRef<ComposerFileDropHandle>(null);
  const conversationFileDrop = useConversationFileDrop(composerFileDropHandle);

  useEffect(() => {
    setModelSelection(thread?.modelSelection ?? null);
  }, [thread?.modelSelection]);

  const handleSideTaskEvent = useCallback(
    (event: AgentEvent) => {
      if (!threadId || event.threadId !== threadId) return;
      if (event.payload.type === "approval_requested") {
        onSetThreadActivity(threadId, "approval");
        onMarkThreadActivityRead(threadId);
      }
      if (event.payload.type === "browser_handoff_required") {
        onSetThreadActivity(threadId, "user_action");
        onMarkThreadActivityRead(threadId);
      }
      if (event.payload.type === "user_input_requested") {
        onSetThreadActivity(threadId, "user_action");
      }

      if (event.payload.type === "turn_started" && event.turnId) {
        onSetThreadActivity(threadId, "processing");
        onMarkThreadActivityRead(threadId);
      } else if (event.payload.type === "turn_finished") {
        onSetThreadActivity(threadId, "succeeded");
        onMarkThreadActivityRead(threadId);
      } else if (event.payload.type === "turn_suspended") {
        onSetThreadActivity(threadId, "approval");
        onMarkThreadActivityRead(threadId);
      } else if (event.payload.type === "turn_cancelled") {
        onSetThreadActivity(threadId, null);
      } else if (event.payload.type === "turn_awaiting_input") {
        onSetThreadActivity(threadId, "user_action");
      }

      if (event.payload.type === "error") {
        onSetThreadActivity(threadId, "failed");
        setActionError(friendlyProviderError(event.payload.message));
        onMarkThreadActivityRead(threadId);
      }
    },
    [onMarkThreadActivityRead, onSetThreadActivity, threadId],
  );

  const { controller: sessionController, state: sessionState } =
    useConversationSession(conversationRegistry, threadId, handleSideTaskEvent);
  const messages = sessionState?.messages ?? [];
  const events = sessionState?.events ?? [];
  const loadState = sessionState?.loadState ?? {
    threadId: null,
    status: "idle" as const,
    error: null,
  };
  const isSending = sessionState?.sending ?? false;
  const activeTurnId = sessionState?.activeTurnId ?? null;
  const pendingTurnFeedback = sessionState?.pendingTurnFeedback ?? null;
  const queuedMessageCount = sessionState?.queuedMessageCount ?? 0;
  const pendingApprovalIds = sessionState?.pendingApprovalIds ?? [];
  const pendingUserInput = sessionState?.pendingUserInput ?? [];
  const decidingApprovalId = sessionState?.decidingApprovalId ?? null;
  const approvalError = sessionState?.approvalError ?? null;
  const submittingUserInputId = sessionState?.submittingUserInputId ?? null;
  const userInputError = sessionState?.userInputError ?? null;
  const sessionError = sessionState?.commandError ?? null;
  const activityMetrics = useMemo(() => {
    const contextWindow = resolveThreadModelContextWindow(
      settings?.providers ?? [],
      settings?.activeProviderId,
      modelSelection,
    );
    return deriveConversationMetrics(
      events,
      modelSelection,
      contextWindow?.contextWindowTokens,
    );
  }, [events, modelSelection, settings?.activeProviderId, settings?.providers]);

  useEffect(() => {
    if (!threadId || sessionState?.loadState.status !== "ready") return;
    const activityStatus =
      sessionState.pendingApprovalIds.length > 0
        ? "approval"
        : resolveThreadActivityStatus(sessionState.turnStatus);
    if (!activityStatus) return;
    onSetThreadActivity(threadId, activityStatus);
    onMarkThreadActivityRead(threadId);
  }, [
    onMarkThreadActivityRead,
    onSetThreadActivity,
    sessionState?.loadState.status,
    sessionState?.pendingApprovalIds.length,
    sessionState?.turnStatus,
    threadId,
  ]);

  const pendingApprovalQueue = useMemo(
    () =>
      events
        .filter(
          (event): event is AgentEvent & { payload: ApprovalRequest } =>
            event.payload.type === "approval_requested" &&
            pendingApprovalIds.includes(event.payload.approval_id),
        )
        .sort((left, right) => left.seq - right.seq),
    [events, pendingApprovalIds],
  );
  const activeApproval = pendingApprovalQueue[0]?.payload ?? null;
  const activeUserInput = pendingUserInput[0] ?? null;
  const workForm = useMemo(
    () => resolveComposerWorkForm(events, null),
    [events],
  );

  async function updateThreadTitle(firstPrompt: string) {
    if (!client || !thread || thread.title !== "侧边任务" || !firstPrompt)
      return;
    try {
      if (threadTitleNeedsSummary(firstPrompt)) {
        const result = await client.generateThreadTitle(
          thread.id,
          firstPrompt,
          thread.title,
        );
        if (result.updated) onThreadUpdated(result.thread);
      } else {
        onThreadUpdated(
          await client.updateThread(thread.id, {
            title: threadTitleFromPrompt(firstPrompt),
          }),
        );
      }
    } catch (error) {
      console.warn("OpenTopia could not title the side task", error);
    }
  }

  async function submitSideTaskMessage(
    input: string,
    imageAttachments: InlineImageAttachment[],
    contentParts: InlineMessageContentPart[],
  ): Promise<boolean> {
    const messageText = input.trim();
    if (
      !sessionController ||
      !thread ||
      isSending ||
      activeApproval ||
      activeUserInput ||
      (!messageText &&
        contextSources.length === 0 &&
        selectedSkillIds.length === 0 &&
        imageAttachments.length === 0)
    ) {
      return false;
    }

    const isFirstPrompt = !messages.some((message) => message.role === "user");
    setActionError(null);
    sessionController.clearCommandError();
    onSetThreadActivity(thread.id, "processing");
    const result = await sessionController.send({
      content: messageText,
      sourcePaths: contextSources.map((source) => source.path),
      skillIds: selectedSkillIds,
      collaborationMode,
      imageAttachments,
      contentParts,
    });
    if (!result) {
      onSetThreadActivity(thread.id, "failed");
      onMarkThreadActivityRead(thread.id);
      return false;
    }
    setComposer("");
    setContextSources([]);
    setSelectedSkillIds([]);
    onMarkThreadActivityRead(thread.id);
    if (isFirstPrompt && messageText) void updateThreadTitle(messageText);
    return true;
  }

  async function cancelSideTaskTurn() {
    if (!sessionController) return;
    setActionError(null);
    await sessionController.cancel();
  }

  async function addSideTaskContextSources(files?: File[]) {
    if (!thread) return;
    setActionError(null);
    try {
      const result = files
        ? await getDroppedContextFiles(files)
        : await selectContextFiles({ defaultPath: thread.workspaceRoot });
      if (result.canceled) return;
      setContextSources((current) => {
        const byPath = new Map(
          current.map((source) => [workspaceRootKey(source.path), source]),
        );
        result.files.forEach((source) =>
          byPath.set(workspaceRootKey(source.path), source),
        );
        return [...byPath.values()].slice(0, 20);
      });
    } catch (error) {
      setActionError(`添加来源失败：${errorMessage(error)}`);
    }
  }

  async function changeSideTaskModel(selection: ThreadModelSelection) {
    if (!client || !thread || activeTurnId) return;
    const previous = modelSelection;
    setModelSelection(selection);
    try {
      const updated = await client.setThreadModel(thread.id, selection);
      onThreadUpdated(updated);
    } catch (error) {
      setModelSelection(previous);
      setActionError(`切换模型失败：${errorMessage(error)}`);
    }
  }

  async function decideSideTaskApproval(approvalId: string, approved: boolean) {
    await sessionController?.decideApproval(approvalId, approved);
  }

  async function submitSideTaskUserInput(
    requestId: string,
    response: UserInputResponse,
  ) {
    await sessionController?.respondToUserInput(requestId, response);
  }

  async function undoSideTaskTurn(turnId: string) {
    if (!client || !thread || undoingTurnId || activeTurnId) return;
    if (!window.confirm("撤销这个回合产生的文件修改？")) return;
    setUndoingTurnId(turnId);
    setActionError(null);
    try {
      await client.undoTurnChanges(thread.id, turnId);
    } catch (error) {
      setActionError(`撤销修改失败：${errorMessage(error)}`);
    } finally {
      setUndoingTurnId(null);
    }
  }

  if (!thread || loadState.status === "idle") {
    return <ConversationLoadingState />;
  }

  return (
    <section
      className={`side-task-conversation ${conversationFileDrop.isDraggingFiles ? "is-dragging-files" : ""}`}
      aria-label="侧边任务会话"
      onDragEnter={conversationFileDrop.onDragEnter}
      onDragOver={conversationFileDrop.onDragOver}
      onDragLeave={conversationFileDrop.onDragLeave}
      onDrop={conversationFileDrop.onDrop}
    >
      {conversationFileDrop.isDraggingFiles ? (
        <ConversationFileDropTarget />
      ) : null}
      {loadState.status === "error" ? (
        <ConversationLoadErrorState
          error={loadState.error ?? "无法加载侧边任务"}
          onRetry={() => sessionController?.retry()}
        />
      ) : loadState.status === "loading" ? (
        <ConversationLoadingState />
      ) : (
        <MessageList
          messages={messages}
          events={events}
          activeTurnId={activeTurnId}
          pendingTurnFeedback={pendingTurnFeedback}
          undoingTurnId={undoingTurnId}
          threadId={thread.id}
          artifacts={[]}
          onOpenArtifact={(artifactId) => onOpenArtifact(thread.id, artifactId)}
          onOpenImagePreview={(sourceId, image) =>
            onOpenImagePreview(thread.id, sourceId, image)
          }
          onOpenAttachmentPreview={(source) =>
            onOpenPreview(
              thread.id,
              { type: "attachment", attachmentId: source.id },
              source.name,
            )
          }
          onOpenMarkdownLink={onOpenMarkdownLink}
          onUndoTurn={(turnId) => void undoSideTaskTurn(turnId)}
          onReviewChanges={() => onOpenToolTab("diff")}
          onOpenFileReview={(path) => onOpenFileReview(path)}
          onLoadTurnFilePreview={(turnId, path, offset) =>
            client
              ? client.getTurnFileDiffPreview(thread.id, turnId, path, offset)
              : Promise.reject(new Error("服务尚未连接"))
          }
        />
      )}
      {actionError || sessionError ? (
        <div className="side-task-conversation-error" role="alert">
          <AlertCircle size={14} aria-hidden="true" />
          <span>{actionError ?? sessionError}</span>
        </div>
      ) : null}
      {activeApproval ? (
        <ApprovalDialog
          key={activeApproval.approval_id}
          request={activeApproval}
          queuePosition={1}
          queueLength={pendingApprovalQueue.length}
          isSubmitting={decidingApprovalId === activeApproval.approval_id}
          error={approvalError}
          onDecision={(approved) =>
            void decideSideTaskApproval(activeApproval.approval_id, approved)
          }
        />
      ) : activeUserInput ? (
        <PlanChoiceCard
          key={activeUserInput.request.requestId}
          request={activeUserInput.request}
          isSubmitting={
            submittingUserInputId === activeUserInput.request.requestId
          }
          error={userInputError}
          onSubmit={(response) =>
            void submitSideTaskUserInput(
              activeUserInput.request.requestId,
              response,
            )
          }
          onSkip={() =>
            void submitSideTaskUserInput(activeUserInput.request.requestId, {
              answers: [],
              skipped: true,
            })
          }
          onCancel={() =>
            void submitSideTaskUserInput(activeUserInput.request.requestId, {
              answers: [],
              cancelled: true,
            })
          }
        />
      ) : (
        <Composer
          autoFocus
          sendShortcut={sendShortcut}
          fileDropHandleRef={composerFileDropHandle}
          fileDropScope="conversation"
          value={composer}
          workForm={workForm}
          isSending={isSending}
          isRunning={Boolean(activeTurnId)}
          isCancelling={sessionState?.cancelling ?? false}
          queuedMessageCount={queuedMessageCount}
          metrics={activityMetrics}
          showContextWindowUsage={showContextWindowUsage}
          modelSelection={modelSelection}
          providers={settings?.providers ?? []}
          activeProviderId={settings?.activeProviderId ?? ""}
          permissionMode={settings?.permissionMode ?? "auto"}
          collaborationMode={collaborationMode}
          sandboxMode={settings?.sandbox.sandboxMode ?? "workspace-write"}
          contextSources={contextSources}
          skills={skills}
          selectedSkillIds={selectedSkillIds}
          workspaceRoot={null}
          projectName={null}
          projects={projects}
          onChange={setComposer}
          onSubmit={submitSideTaskMessage}
          onCancel={() => void cancelSideTaskTurn()}
          onPickWorkspace={() => undefined}
          onSelectProject={() => undefined}
          onChangePermissionMode={onChangePermissionMode}
          onChangeCollaborationMode={setCollaborationMode}
          onChangeSandboxMode={onChangeSandboxMode}
          onChangeModelSelection={(selection) =>
            void changeSideTaskModel(selection)
          }
          onOpenSettings={onOpenSettings}
          onAddContextSources={addSideTaskContextSources}
          onRemoveContextSource={(path) =>
            setContextSources((current) =>
              current.filter(
                (source) =>
                  workspaceRootKey(source.path) !== workspaceRootKey(path),
              ),
            )
          }
          onToggleSkill={(skillId) =>
            setSelectedSkillIds((current) =>
              current.includes(skillId)
                ? current.filter((id) => id !== skillId)
                : [...current, skillId],
            )
          }
        />
      )}
    </section>
  );
}
