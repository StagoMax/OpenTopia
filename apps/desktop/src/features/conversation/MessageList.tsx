import {
  Fragment,
  memo,
  useCallback,
  useDeferredValue,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ClipboardEvent as ReactClipboardEvent,
} from "react";
import {
  AlertCircle,
  ArrowDown,
  Bot,
  Check,
  CircleAlert,
  Copy,
  Loader2,
} from "lucide-react";
import {
  PendingTurnStatus,
  TurnActivityTimeline,
  TurnChangeCard,
} from "../../components/TurnActivityTimeline";
import { Button, IconButton } from "../../components/ui";
import type { ImagePreviewSource } from "../../components/PreviewHost";
import { normalizeCopiedText } from "../../clipboardText";
import {
  attachmentsByAssistantMessage,
  stabilizeAttachmentReferences,
} from "../../conversationAttachmentReferences";
import {
  conversationMessageCopyText,
  formatConversationMessageTimestamp,
} from "../../conversationMessageMeta";
import { isConversationScrollNearEnd } from "../../conversationScroll";
import {
  projectConversationEvents,
  type ConversationEventProjection,
} from "../../conversationEventProjection";
import type { PendingTurnFeedback } from "../../threadRunState";
import { friendlyProviderError } from "../../providerErrors";
import {
  activeProviderRequestPhase,
  hasPendingToolCall,
} from "../../turnActivityStatus";
import type {
  AgentEvent,
  ArtifactDescriptor,
  ContextSourceRef,
  Message,
  TurnFileChange,
  TurnFileDiffPreview,
  ToolResult,
} from "../../types";
import { ConversationLoadingIndicator } from "./ConversationLoadingIndicator";
import { MessagePartView } from "./MessagePartView";

const emptyAttachmentSources: ContextSourceRef[] = [];

export type MessageListProps = {
  messages: Message[];
  events: AgentEvent[];
  activeTurnId: string | null;
  pendingTurnFeedback: PendingTurnFeedback | null;
  syncing?: boolean;
  syncError?: string | null;
  hasOlderMessages?: boolean;
  loadingOlderMessages?: boolean;
  olderMessagesError?: string | null;
  undoingTurnId: string | null;
  threadId: string;
  artifacts: ArtifactDescriptor[];
  onOpenArtifact(artifactId: string): void;
  onOpenImagePreview(sourceId: string, image: ImagePreviewSource): void;
  onOpenAttachmentPreview(source: ContextSourceRef): void;
  onOpenMarkdownLink(href: string, baseWorkspacePath?: string | null): void;
  onImplementProposedPlan(): void;
  isProposedPlanActionDisabled: boolean;
  onUndoTurn(turnId: string): void;
  onReviewChanges(): void;
  onOpenFileReview(path: string, file: TurnFileChange): void;
  onLoadTurnFilePreview(
    turnId: string,
    path: string,
    offset?: number,
  ): Promise<TurnFileDiffPreview>;
  onLoadToolResultDetail?(eventId: string): Promise<ToolResult>;
  onLoadOlderMessages?(): Promise<void>;
  onRetrySync?(): void;
};

export function MessageList({
  messages,
  events: latestEvents,
  activeTurnId,
  pendingTurnFeedback,
  syncing = false,
  syncError = null,
  hasOlderMessages = false,
  loadingOlderMessages = false,
  olderMessagesError = null,
  undoingTurnId,
  threadId,
  artifacts,
  onOpenArtifact,
  onOpenImagePreview,
  onOpenAttachmentPreview,
  onOpenMarkdownLink,
  onImplementProposedPlan,
  isProposedPlanActionDisabled,
  onUndoTurn,
  onReviewChanges,
  onOpenFileReview,
  onLoadTurnFilePreview,
  onLoadToolResultDetail,
  onLoadOlderMessages,
  onRetrySync,
}: MessageListProps) {
  // Tool events originate in an external store, whose updates React must
  // process synchronously. Keep the previous timeline during that urgent pass
  // and let React build the event-heavy replacement in an interruptible render
  // so persistent chrome animations remain responsive.
  const events = useDeferredValue(latestEvents);
  const visibleMessages = useMemo(
    () =>
      messages.filter(
        (message) => message.role === "user" || message.role === "assistant",
      ),
    [messages],
  );
  const attachmentSourcesCacheRef = useRef<Map<string, ContextSourceRef[]>>(
    new Map(),
  );
  const attachmentSourcesByAssistantMessage = useMemo(() => {
    const next = stabilizeAttachmentReferences(
      attachmentSourcesCacheRef.current,
      attachmentsByAssistantMessage(messages, events),
    );
    attachmentSourcesCacheRef.current = next;
    return next;
  }, [events, messages]);
  const messageListRef = useRef<HTMLDivElement>(null);
  const messageListContentRef = useRef<HTMLDivElement>(null);
  const previousScrollHeightRef = useRef<number | null>(null);
  const olderLoadMessageCountRef = useRef<number | null>(null);
  const suppressNextPinnedScrollRef = useRef(false);
  const conversationPinnedToEndRef = useRef(true);
  const [showScrollToEnd, setShowScrollToEnd] = useState(false);
  const actionableProposedPlanMessageId = useMemo(() => {
    const latestMessage = visibleMessages[visibleMessages.length - 1];
    return latestMessage?.parts.some((part) => part.type === "proposed_plan")
      ? latestMessage.id
      : null;
  }, [visibleMessages]);
  const eventProjectionCacheRef = useRef<ConversationEventProjection | null>(
    null,
  );
  const {
    eventsByTurn,
    turnIdsByUserMessage,
    turnIdsByAssistantMessage,
    changeSetsByTurn,
    revertedTurnIds,
    orphanContextActivityTurnIds,
    orphanTurnErrors,
    turnsWithAssistantCards,
    settledTurnIds,
  } = useMemo(() => {
    const next = projectConversationEvents(
      events,
      eventProjectionCacheRef.current ?? undefined,
    );
    eventProjectionCacheRef.current = next;
    return next;
  }, [events]);
  const visibleMessageIds = useMemo(
    () => new Set(visibleMessages.map((message) => message.id)),
    [visibleMessages],
  );
  const activeTurnIsAnchored =
    activeTurnId !== null &&
    [...turnIdsByUserMessage.entries()].some(
      ([userMessageId, turnIds]) =>
        visibleMessageIds.has(userMessageId) && turnIds.includes(activeTurnId),
    );
  const pendingTurnIsAnchored =
    pendingTurnFeedback !== null &&
    visibleMessageIds.has(pendingTurnFeedback.userMessageId) &&
    events.some(
      (event) =>
        event.payload.type === "turn_started" &&
        (pendingTurnFeedback.turnId
          ? event.turnId === pendingTurnFeedback.turnId
          : event.payload.user_message_id ===
            pendingTurnFeedback.userMessageId),
    );
  const showPendingTurnStatus =
    pendingTurnFeedback !== null &&
    visibleMessageIds.has(pendingTurnFeedback.userMessageId) &&
    !pendingTurnIsAnchored;
  const activeTurnEvents =
    activeTurnId === null ? [] : (eventsByTurn.get(activeTurnId) ?? []);
  const activeTurnUserMessageId =
    activeTurnId === null
      ? null
      : pendingTurnFeedback?.turnId === activeTurnId
        ? pendingTurnFeedback.userMessageId
        : ([...turnIdsByUserMessage.entries()].find(([, turnIds]) =>
            turnIds.includes(activeTurnId),
          )?.[0] ?? null);
  const providerRequestPhase = activeTurnIsAnchored
    ? activeProviderRequestPhase(activeTurnEvents)
    : null;
  const showActiveProcessingStatus =
    activeTurnIsAnchored &&
    providerRequestPhase === null &&
    !hasPendingToolCall(activeTurnEvents);
  const showTrailingTurnStatus = showPendingTurnStatus;

  useLayoutEffect(() => {
    if (loadingOlderMessages) return;
    const previousScrollHeight = previousScrollHeightRef.current;
    const list = messageListRef.current;
    if (previousScrollHeight === null || !list) return;
    list.scrollTop += list.scrollHeight - previousScrollHeight;
    suppressNextPinnedScrollRef.current =
      olderLoadMessageCountRef.current !== null &&
      messages.length > olderLoadMessageCountRef.current;
    previousScrollHeightRef.current = null;
    olderLoadMessageCountRef.current = null;
  }, [loadingOlderMessages, messages.length]);

  const loadOlderMessages = useCallback(() => {
    if (!onLoadOlderMessages) return;
    previousScrollHeightRef.current =
      messageListRef.current?.scrollHeight ?? null;
    olderLoadMessageCountRef.current = messages.length;
    void onLoadOlderMessages();
  }, [messages.length, onLoadOlderMessages]);

  const updateScrollToEndVisibility = useCallback(() => {
    const list = messageListRef.current;
    if (!list) return;
    const isNearEnd = isConversationScrollNearEnd(list);
    conversationPinnedToEndRef.current = isNearEnd;
    setShowScrollToEnd(!isNearEnd);
  }, []);

  useEffect(() => {
    const list = messageListRef.current;
    if (!list) return;
    list.addEventListener("scroll", updateScrollToEndVisibility, {
      passive: true,
    });
    window.addEventListener("resize", updateScrollToEndVisibility);
    updateScrollToEndVisibility();
    return () => {
      list.removeEventListener("scroll", updateScrollToEndVisibility);
      window.removeEventListener("resize", updateScrollToEndVisibility);
    };
  }, [updateScrollToEndVisibility]);

  useLayoutEffect(() => {
    const list = messageListRef.current;
    if (suppressNextPinnedScrollRef.current) {
      suppressNextPinnedScrollRef.current = false;
    } else if (list && conversationPinnedToEndRef.current) {
      list.scrollTop = list.scrollHeight;
    }
    updateScrollToEndVisibility();
  }, [messages, updateScrollToEndVisibility]);

  useEffect(() => {
    const content = messageListContentRef.current;
    if (!content) return;
    let frame: number | null = null;
    const observer = new ResizeObserver(() => {
      if (frame !== null) return;
      frame = window.requestAnimationFrame(() => {
        frame = null;
        const list = messageListRef.current;
        if (!list) return;
        if (conversationPinnedToEndRef.current) {
          list.scrollTop = list.scrollHeight;
        }
        updateScrollToEndVisibility();
      });
    });
    observer.observe(content);
    return () => {
      observer.disconnect();
      if (frame !== null) window.cancelAnimationFrame(frame);
    };
  }, [updateScrollToEndVisibility]);

  const scrollToEnd = useCallback(() => {
    const list = messageListRef.current;
    if (!list) return;
    conversationPinnedToEndRef.current = true;
    list.scrollTo({ top: list.scrollHeight, behavior: "smooth" });
  }, []);

  const renderTurnChangeCard = (turnId: string) => {
    const changeSet = changeSetsByTurn.get(turnId);
    if (!changeSet || activeTurnId === turnId || !settledTurnIds.has(turnId)) {
      return null;
    }
    return (
      <TurnChangeCard
        key={`turn-change-card-${turnId}`}
        changeSet={changeSet}
        isWorkspaceBusy={Boolean(activeTurnId)}
        isUndoing={undoingTurnId === turnId}
        isReverted={revertedTurnIds.has(turnId)}
        onUndo={() => onUndoTurn(turnId)}
        onReview={onReviewChanges}
        onOpenFileReview={onOpenFileReview}
        onLoadFilePreview={(path, offset) =>
          onLoadTurnFilePreview(turnId, path, offset)
        }
      />
    );
  };
  return (
    <div className="conversation-scroll-shell">
      <div
        className={`message-list ${syncing ? "is-syncing" : ""}`.trim()}
        ref={messageListRef}
        aria-busy={syncing || loadingOlderMessages || showTrailingTurnStatus}
        onCopy={trimCopiedSelection}
      >
        {syncing ? (
          <div className="conversation-refresh-overlay">
            <ConversationLoadingIndicator label="正在同步最新内容" />
          </div>
        ) : null}
        <div
          className={`message-list-content ${
            visibleMessages.length === 0 && !showTrailingTurnStatus
              ? "is-empty"
              : ""
          }`.trim()}
          data-text-context-menu={
            visibleMessages.length > 0 || showTrailingTurnStatus
              ? "conversation-history"
              : undefined
          }
          ref={messageListContentRef}
        >
          {syncError ? (
            <div className="conversation-sync-error" role="alert">
              <span>同步失败，当前显示的是上次快照：{syncError}</span>
              {onRetrySync ? (
                <Button size="compact" variant="quiet" onClick={onRetrySync}>
                  重试
                </Button>
              ) : null}
            </div>
          ) : null}
          {hasOlderMessages || olderMessagesError ? (
            <div className="conversation-history-pagination">
              {hasOlderMessages ? (
                <Button
                  size="compact"
                  variant="quiet"
                  disabled={loadingOlderMessages || syncing}
                  onClick={loadOlderMessages}
                >
                  {loadingOlderMessages ? (
                    <Loader2 aria-hidden="true" size={14} className="spin" />
                  ) : null}
                  加载更早消息
                </Button>
              ) : null}
              {olderMessagesError ? (
                <span role="alert">{olderMessagesError}</span>
              ) : null}
            </div>
          ) : null}
          {visibleMessages.length === 0 && !showTrailingTurnStatus ? (
            <div className="empty-thread">
              <Bot size={42} />
              <h2>等待第一个任务指令</h2>
              <p>当前任务尚未产生消息。</p>
            </div>
          ) : (
            visibleMessages.map((message) => {
              const turnIds =
                message.role === "user"
                  ? (turnIdsByUserMessage.get(message.id) ?? [])
                  : [];
              const resultTurnIds =
                message.role === "assistant"
                  ? (turnIdsByAssistantMessage.get(message.id) ?? [])
                  : [];
              return (
                <Fragment key={message.id}>
                  <MessageBubble
                    attachmentSources={
                      attachmentSourcesByAssistantMessage.get(message.id) ??
                      emptyAttachmentSources
                    }
                    message={message}
                    threadId={threadId}
                    artifacts={artifacts}
                    onOpenArtifact={onOpenArtifact}
                    onOpenImagePreview={onOpenImagePreview}
                    onOpenAttachmentPreview={onOpenAttachmentPreview}
                    onOpenMarkdownLink={onOpenMarkdownLink}
                    onImplementProposedPlan={
                      message.id === actionableProposedPlanMessageId
                        ? onImplementProposedPlan
                        : undefined
                    }
                    isProposedPlanActionDisabled={isProposedPlanActionDisabled}
                  />
                  {turnIds.map((turnId) => (
                    <Fragment key={turnId}>
                      <TurnActivityTimeline
                        events={eventsByTurn.get(turnId) ?? []}
                        isActive={activeTurnId === turnId}
                        formatError={friendlyProviderError}
                        onOpenMarkdownLink={onOpenMarkdownLink}
                        onLoadToolResultDetail={onLoadToolResultDetail}
                      />
                      {!turnsWithAssistantCards.has(turnId) &&
                        renderTurnChangeCard(turnId)}
                    </Fragment>
                  ))}
                  {resultTurnIds.map(renderTurnChangeCard)}
                  {showPendingTurnStatus &&
                  pendingTurnFeedback?.userMessageId === message.id ? (
                    <PendingTurnStatus
                      key={`pending-${pendingTurnFeedback.startedAt}`}
                      phase="processing"
                      threadId={pendingTurnFeedback.threadId}
                      turnId={pendingTurnFeedback.turnId}
                    />
                  ) : null}
                  {activeTurnUserMessageId === message.id &&
                  activeTurnId &&
                  (providerRequestPhase || showActiveProcessingStatus) ? (
                    <PendingTurnStatus
                      key={`active-${activeTurnId}`}
                      phase={providerRequestPhase ?? "processing"}
                      threadId={threadId}
                      turnId={activeTurnId}
                    />
                  ) : null}
                </Fragment>
              );
            })
          )}
          {orphanContextActivityTurnIds.map((turnId) => {
            const activityEvents = eventsByTurn.get(turnId) ?? [];
            return (
              <TurnActivityTimeline
                key={`context-compaction-${turnId}`}
                events={activityEvents}
                isActive={!contextCompactionActivityFinished(activityEvents)}
                standalone
                formatError={friendlyProviderError}
                onOpenMarkdownLink={onOpenMarkdownLink}
                onLoadToolResultDetail={onLoadToolResultDetail}
              />
            );
          })}
          {orphanTurnErrors.map((event) => (
            <article
              className="message assistant turn-error-message"
              key={event.id}
            >
              <div className="message-body" role="alert">
                <AlertCircle size={15} aria-hidden="true" />
                <span>
                  {event.payload.type === "error"
                    ? friendlyProviderError(event.payload.message)
                    : "Agent 请求失败"}
                </span>
              </div>
            </article>
          ))}
        </div>
      </div>
      {showScrollToEnd ? (
        <IconButton
          className="conversation-scroll-to-end"
          variant="secondary"
          aria-label="滚动到对话末尾"
          title="滚动到对话末尾"
          onClick={scrollToEnd}
        >
          <ArrowDown size={18} aria-hidden="true" />
        </IconButton>
      ) : null}
    </div>
  );
}

function contextCompactionActivityFinished(events: AgentEvent[]) {
  return events.some((event) => {
    const payload = event.payload;
    return (
      payload.type === "context_compacted" ||
      (payload.type === "context_warning" &&
        payload.stage.includes("compaction"))
    );
  });
}

function trimCopiedSelection(event: ReactClipboardEvent<HTMLDivElement>) {
  const selected = window.getSelection()?.toString() ?? "";
  const trimmed = normalizeCopiedText(selected);
  if (!trimmed || trimmed === selected) return;
  event.clipboardData.setData("text/plain", trimmed);
  event.preventDefault();
}

type ImageLightboxAttachment = {
  name: string;
  previewUrl: string;
};

const MessageBubble = memo(function MessageBubble({
  attachmentSources,
  message,
  threadId,
  artifacts,
  onOpenArtifact,
  onOpenImagePreview,
  onOpenAttachmentPreview,
  onOpenMarkdownLink,
  onImplementProposedPlan,
  isProposedPlanActionDisabled,
}: {
  attachmentSources: ContextSourceRef[];
  message: Message;
  threadId: string;
  artifacts: ArtifactDescriptor[];
  onOpenArtifact(artifactId: string): void;
  onOpenImagePreview(sourceId: string, image: ImagePreviewSource): void;
  onOpenAttachmentPreview(source: ContextSourceRef): void;
  onOpenMarkdownLink(href: string): void;
  onImplementProposedPlan?(): void;
  isProposedPlanActionDisabled: boolean;
}) {
  const [copyStatus, setCopyStatus] = useState<"idle" | "copied" | "error">(
    "idle",
  );
  const renderedParts = useMemo(() => {
    const referencedImageIds = new Set(
      message.parts.flatMap((part) =>
        part.type === "image_ref" ? [part.image_id] : [],
      ),
    );
    const imagesById = new Map(
      message.parts.flatMap((part) =>
        part.type === "image" && part.id ? [[part.id, part] as const] : [],
      ),
    );
    let nextImagePreviewIndex = 0;

    return message.parts
      .filter(
        (part) =>
          part.type !== "turn_context" &&
          part.type !== "tool_call" &&
          part.type !== "tool_result" &&
          !(
            part.type === "image" &&
            part.id &&
            referencedImageIds.has(part.id)
          ),
      )
      .map((part) => {
        const referencedImage =
          part.type === "image_ref" ? imagesById.get(part.image_id) : undefined;
        const previewImage = part.type === "image" ? part : referencedImage;
        const previewIndex = previewImage ? nextImagePreviewIndex++ : null;
        return { part, referencedImage, previewImage, previewIndex };
      });
  }, [message.parts]);
  const [imagePreviews, setImagePreviews] = useState<ImageLightboxAttachment[]>(
    [],
  );
  const copyText = useMemo(
    () => conversationMessageCopyText(message.parts),
    [message.parts],
  );
  const timestamp = useMemo(
    () => formatConversationMessageTimestamp(message.createdAt),
    [message.createdAt],
  );

  useEffect(() => {
    if (copyStatus === "idle") return;
    const timer = window.setTimeout(() => setCopyStatus("idle"), 1600);
    return () => window.clearTimeout(timer);
  }, [copyStatus]);

  useLayoutEffect(() => {
    const nextImagePreviews = renderedParts.flatMap(({ previewImage }) => {
      if (!previewImage) return [];
      const contentType =
        previewImage.contentType ||
        (previewImage as typeof previewImage & { content_type?: string })
          .content_type ||
        "application/octet-stream";
      return [
        {
          name: previewImage.name || "图片",
          previewUrl: URL.createObjectURL(
            new Blob([new Uint8Array(previewImage.data)], {
              type: contentType,
            }),
          ),
        },
      ];
    });
    setImagePreviews(nextImagePreviews);
    return () => {
      nextImagePreviews.forEach(({ previewUrl }) =>
        URL.revokeObjectURL(previewUrl),
      );
    };
  }, [renderedParts]);

  if (renderedParts.length === 0) return null;

  return (
    <article className={`message ${message.role}`}>
      <div className="message-content">
        <div className="message-body">
          {renderedParts.map(
            ({ part, referencedImage, previewImage, previewIndex }, index) => (
              <MessagePartView
                attachmentSources={attachmentSources}
                key={index}
                messageId={message.id}
                part={part}
                referencedImage={referencedImage}
                imagePreviewUrl={
                  previewIndex === null
                    ? undefined
                    : imagePreviews[previewIndex]?.previewUrl
                }
                onPreviewImage={
                  previewIndex === null || !previewImage
                    ? undefined
                    : () =>
                        onOpenImagePreview(
                          `${message.id}:${previewIndex}`,
                          previewImage,
                        )
                }
                role={message.role}
                threadId={threadId}
                artifacts={artifacts}
                onOpenArtifact={onOpenArtifact}
                onOpenAttachmentPreview={onOpenAttachmentPreview}
                onOpenMarkdownLink={onOpenMarkdownLink}
                onImplementProposedPlan={onImplementProposedPlan}
                isProposedPlanActionDisabled={isProposedPlanActionDisabled}
              />
            ),
          )}
        </div>
        <div className="message-actions">
          {timestamp ? (
            <time dateTime={message.createdAt} title={timestamp.title}>
              {timestamp.label}
            </time>
          ) : null}
          {copyText ? (
            <IconButton
              className="message-copy-button"
              size="compact"
              variant="quiet"
              aria-label={
                copyStatus === "copied"
                  ? "消息已复制"
                  : copyStatus === "error"
                    ? "复制失败，重试"
                    : "复制消息"
              }
              title={
                copyStatus === "copied"
                  ? "已复制"
                  : copyStatus === "error"
                    ? "复制失败，点击重试"
                    : "复制消息"
              }
              data-state={copyStatus}
              onClick={() => {
                void (async () => {
                  try {
                    if (!navigator.clipboard?.writeText) {
                      throw new Error("Clipboard API unavailable");
                    }
                    await navigator.clipboard.writeText(copyText);
                    setCopyStatus("copied");
                  } catch {
                    setCopyStatus("error");
                  }
                })();
              }}
            >
              {copyStatus === "copied" ? (
                <Check size={14} aria-hidden="true" />
              ) : copyStatus === "error" ? (
                <CircleAlert size={14} aria-hidden="true" />
              ) : (
                <Copy size={14} aria-hidden="true" />
              )}
            </IconButton>
          ) : null}
          <span className="ot-sr-only" aria-live="polite">
            {copyStatus === "copied"
              ? "消息已复制到剪贴板"
              : copyStatus === "error"
                ? "消息复制失败"
                : ""}
          </span>
        </div>
      </div>
    </article>
  );
});
