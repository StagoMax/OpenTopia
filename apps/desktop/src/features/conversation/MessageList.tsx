import {
  Fragment,
  memo,
  useCallback,
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
} from "lucide-react";
import {
  PendingTurnStatus,
  TurnActivityTimeline,
  TurnChangeCard,
} from "../../components/TurnActivityTimeline";
import { IconButton } from "../../components/ui";
import type { ImagePreviewSource } from "../../components/PreviewHost";
import { normalizeCopiedText } from "../../clipboardText";
import { attachmentsByAssistantMessage } from "../../conversationAttachmentReferences";
import {
  conversationMessageCopyText,
  formatConversationMessageTimestamp,
} from "../../conversationMessageMeta";
import { isConversationScrollNearEnd } from "../../conversationScroll";
import type { PendingTurnFeedback } from "../../conversationSession";
import { friendlyProviderError } from "../../providerErrors";
import { shouldShowRecordedTurnChanges } from "../../turnChangeOwnership";
import {
  hasPendingProviderRequest,
  hasPendingToolCall,
} from "../../turnActivityStatus";
import type {
  AgentEvent,
  ArtifactDescriptor,
  ContextSourceRef,
  Message,
  TurnChangeSet,
  TurnFileChange,
  TurnFileDiffPreview,
} from "../../types";
import { MessagePartView } from "./MessagePartView";

const initialRenderedMessageCount = 12;
const messageRenderBatchSize = 12;

export function MessageList({
  messages,
  events,
  activeTurnId,
  pendingTurnFeedback,
  undoingTurnId,
  threadId,
  artifacts,
  onOpenArtifact,
  onOpenImagePreview,
  onOpenAttachmentPreview,
  onOpenMarkdownLink,
  onUndoTurn,
  onReviewChanges,
  onOpenFileReview,
  onLoadTurnFilePreview,
}: {
  messages: Message[];
  events: AgentEvent[];
  activeTurnId: string | null;
  pendingTurnFeedback: PendingTurnFeedback | null;
  undoingTurnId: string | null;
  threadId: string;
  artifacts: ArtifactDescriptor[];
  onOpenArtifact(artifactId: string): void;
  onOpenImagePreview(sourceId: string, image: ImagePreviewSource): void;
  onOpenAttachmentPreview(source: ContextSourceRef): void;
  onOpenMarkdownLink(href: string, baseWorkspacePath?: string | null): void;
  onUndoTurn(turnId: string): void;
  onReviewChanges(): void;
  onOpenFileReview(path: string, file: TurnFileChange): void;
  onLoadTurnFilePreview(
    turnId: string,
    path: string,
    offset?: number,
  ): Promise<TurnFileDiffPreview>;
}) {
  const visibleMessages = useMemo(
    () =>
      messages.filter(
        (message) => message.role === "user" || message.role === "assistant",
      ),
    [messages],
  );
  const attachmentSourcesByAssistantMessage = useMemo(
    () => attachmentsByAssistantMessage(messages, events),
    [events, messages],
  );
  const [renderedMessageCount, setRenderedMessageCount] = useState(
    initialRenderedMessageCount,
  );
  const messageListRef = useRef<HTMLDivElement>(null);
  const messageListContentRef = useRef<HTMLDivElement>(null);
  const previousScrollHeightRef = useRef<number | null>(null);
  const conversationPinnedToEndRef = useRef(true);
  const [showScrollToEnd, setShowScrollToEnd] = useState(false);
  const renderedMessages = visibleMessages.slice(-renderedMessageCount);
  const hasPendingMessages = renderedMessages.length < visibleMessages.length;
  const {
    eventsByTurn,
    turnIdsByUserMessage,
    turnIdsByAssistantMessage,
    changeSetsByTurn,
    revertedTurnIds,
    orphanTurnErrors,
    turnsWithAssistantCards,
  } = useMemo(() => {
    const eventsByTurn = new Map<string, AgentEvent[]>();
    const turnIdsByUserMessage = new Map<string, string[]>();
    const turnIdsByAssistantMessage = new Map<string, string[]>();
    const changeSetsByTurn = new Map<string, TurnChangeSet>();
    const revertedTurnIds = new Set<string>();
    for (const event of events) {
      if (event.turnId) {
        const current = eventsByTurn.get(event.turnId) ?? [];
        current.push(event);
        eventsByTurn.set(event.turnId, current);
      }
      if (event.turnId && event.payload.type === "turn_started") {
        const turnIds =
          turnIdsByUserMessage.get(event.payload.user_message_id) ?? [];
        if (!turnIds.includes(event.turnId)) turnIds.push(event.turnId);
        turnIdsByUserMessage.set(event.payload.user_message_id, turnIds);
      }
      if (event.turnId && event.payload.type === "assistant_message") {
        const turnIds =
          turnIdsByAssistantMessage.get(event.payload.message.id) ?? [];
        if (!turnIds.includes(event.turnId)) turnIds.push(event.turnId);
        turnIdsByAssistantMessage.set(event.payload.message.id, turnIds);
      }
      if (
        event.turnId &&
        event.payload.type === "turn_changes_recorded" &&
        shouldShowRecordedTurnChanges(events, event.turnId)
      ) {
        changeSetsByTurn.set(event.turnId, event.payload.change_set);
        if (event.payload.change_set.revertedAt) {
          revertedTurnIds.add(event.turnId);
        }
      }
      if (event.payload.type === "turn_undo_completed") {
        revertedTurnIds.add(event.payload.target_turn_id);
      }
    }
    const anchoredTurnIds = new Set(
      [...turnIdsByUserMessage.values()].flatMap((turnIds) => turnIds),
    );
    const orphanTurnErrors = events.filter(
      (event) =>
        event.payload.type === "error" &&
        (!event.turnId || !anchoredTurnIds.has(event.turnId)),
    );
    const turnsWithAssistantCards = new Set(
      [...turnIdsByAssistantMessage.values()].flatMap((turnIds) => turnIds),
    );
    return {
      eventsByTurn,
      turnIdsByUserMessage,
      turnIdsByAssistantMessage,
      changeSetsByTurn,
      revertedTurnIds,
      orphanTurnErrors,
      turnsWithAssistantCards,
    };
  }, [events]);
  const pendingTurnIsAnchored = pendingTurnFeedback
    ? events.some(
        (event) =>
          event.payload.type === "turn_started" &&
          (pendingTurnFeedback.turnId
            ? event.turnId === pendingTurnFeedback.turnId
            : event.createdAt >= pendingTurnFeedback.startedAt),
      )
    : false;
  const showPendingTurnStatus =
    pendingTurnFeedback !== null && !pendingTurnIsAnchored;
  const activeTurnEvents =
    activeTurnId === null ? [] : (eventsByTurn.get(activeTurnId) ?? []);
  const showModelThinkingStatus =
    activeTurnId !== null && hasPendingProviderRequest(activeTurnEvents);
  const showActiveProcessingStatus =
    activeTurnId !== null &&
    !showModelThinkingStatus &&
    !hasPendingToolCall(activeTurnEvents);
  const showTrailingTurnStatus =
    showPendingTurnStatus ||
    showModelThinkingStatus ||
    showActiveProcessingStatus;

  useEffect(() => {
    if (!hasPendingMessages) return;
    const frame = window.requestAnimationFrame(() => {
      previousScrollHeightRef.current =
        messageListRef.current?.scrollHeight ?? null;
      setRenderedMessageCount((current) =>
        Math.min(current + messageRenderBatchSize, visibleMessages.length),
      );
    });
    return () => window.cancelAnimationFrame(frame);
  }, [hasPendingMessages, visibleMessages.length]);

  useLayoutEffect(() => {
    const previousScrollHeight = previousScrollHeightRef.current;
    const list = messageListRef.current;
    if (previousScrollHeight === null || !list) return;
    list.scrollTop += list.scrollHeight - previousScrollHeight;
    previousScrollHeightRef.current = null;
  }, [renderedMessageCount]);

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
    if (list && conversationPinnedToEndRef.current) {
      list.scrollTop = list.scrollHeight;
    }
    updateScrollToEndVisibility();
  }, [events, messages, renderedMessageCount, updateScrollToEndVisibility]);

  useEffect(() => {
    const content = messageListContentRef.current;
    if (!content) return;
    const observer = new ResizeObserver(() => {
      const list = messageListRef.current;
      if (!list) return;
      if (conversationPinnedToEndRef.current) {
        list.scrollTop = list.scrollHeight;
      }
      updateScrollToEndVisibility();
    });
    observer.observe(content);
    return () => observer.disconnect();
  }, [updateScrollToEndVisibility]);

  const scrollToEnd = useCallback(() => {
    const list = messageListRef.current;
    if (!list) return;
    conversationPinnedToEndRef.current = true;
    list.scrollTo({ top: list.scrollHeight, behavior: "smooth" });
  }, []);

  const renderTurnChangeCard = (turnId: string) => {
    const changeSet = changeSetsByTurn.get(turnId);
    if (!changeSet) return null;
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
        className="message-list"
        ref={messageListRef}
        aria-busy={hasPendingMessages || showTrailingTurnStatus}
        onCopy={trimCopiedSelection}
      >
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
          {visibleMessages.length === 0 && !showTrailingTurnStatus ? (
            <div className="empty-thread">
              <Bot size={42} />
              <h2>等待第一个任务指令</h2>
              <p>当前任务尚未产生消息。</p>
            </div>
          ) : (
            renderedMessages.map((message) => {
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
                      attachmentSourcesByAssistantMessage.get(message.id) ?? []
                    }
                    message={message}
                    threadId={threadId}
                    artifacts={artifacts}
                    onOpenArtifact={onOpenArtifact}
                    onOpenImagePreview={onOpenImagePreview}
                    onOpenAttachmentPreview={onOpenAttachmentPreview}
                    onOpenMarkdownLink={onOpenMarkdownLink}
                  />
                  {turnIds.map((turnId) => (
                    <Fragment key={turnId}>
                      <TurnActivityTimeline
                        events={eventsByTurn.get(turnId) ?? []}
                        isActive={activeTurnId === turnId}
                        formatError={friendlyProviderError}
                        onOpenMarkdownLink={onOpenMarkdownLink}
                      />
                      {!turnsWithAssistantCards.has(turnId) &&
                        renderTurnChangeCard(turnId)}
                    </Fragment>
                  ))}
                  {resultTurnIds.map(renderTurnChangeCard)}
                </Fragment>
              );
            })
          )}
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
          {showModelThinkingStatus && activeTurnId ? (
            <PendingTurnStatus
              key={`model-thinking-${activeTurnId}`}
              phase="thinking"
              threadId={threadId}
              turnId={activeTurnId}
            />
          ) : showActiveProcessingStatus && activeTurnId ? (
            <PendingTurnStatus
              key={`model-processing-${activeTurnId}`}
              phase="processing"
              threadId={threadId}
              turnId={activeTurnId}
            />
          ) : showPendingTurnStatus && pendingTurnFeedback ? (
            <PendingTurnStatus
              key={pendingTurnFeedback.startedAt}
              phase="processing"
              threadId={pendingTurnFeedback.threadId}
              turnId={pendingTurnFeedback.turnId}
            />
          ) : null}
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
}: {
  attachmentSources: ContextSourceRef[];
  message: Message;
  threadId: string;
  artifacts: ArtifactDescriptor[];
  onOpenArtifact(artifactId: string): void;
  onOpenImagePreview(sourceId: string, image: ImagePreviewSource): void;
  onOpenAttachmentPreview(source: ContextSourceRef): void;
  onOpenMarkdownLink(href: string): void;
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
