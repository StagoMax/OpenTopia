import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const timelineSource = readFileSync(
  new URL("./components/TurnActivityTimeline.tsx", import.meta.url),
  "utf8",
);
const toolCardSource = readFileSync(
  new URL("./components/ToolActivityCard.tsx", import.meta.url),
  "utf8",
);
const messageListSource = readFileSync(
  new URL("./features/conversation/MessageList.tsx", import.meta.url),
  "utf8",
);
const conversationHeaderSource = readFileSync(
  new URL("./features/conversation/ConversationHeader.tsx", import.meta.url),
  "utf8",
);
const loadingIndicatorSource = readFileSync(
  new URL(
    "./features/conversation/ConversationLoadingIndicator.tsx",
    import.meta.url,
  ),
  "utf8",
);
const sidebarRowSource = readFileSync(
  new URL("./features/sidebar/SidebarThreadRow.tsx", import.meta.url),
  "utf8",
);
const appSource = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
const conversationHookSource = readFileSync(
  new URL("./useConversationSession.ts", import.meta.url),
  "utf8",
);
const conversationControllerSource = readFileSync(
  new URL("./conversationSessionController.ts", import.meta.url),
  "utf8",
);
const activityStoreSource = readFileSync(
  new URL("./threadActivityStore.ts", import.meta.url),
  "utf8",
);
const activityHookSource = readFileSync(
  new URL("./useThreadActivityStore.ts", import.meta.url),
  "utf8",
);
const liveMessageListSource = readFileSync(
  new URL(
    "./features/conversation/LiveConversationMessageList.tsx",
    import.meta.url,
  ),
  "utf8",
);
const liveComposerSource = readFileSync(
  new URL("./features/composer/LiveConversationComposer.tsx", import.meta.url),
  "utf8",
);
const composerSource = readFileSync(
  new URL("./features/composer/Composer.tsx", import.meta.url),
  "utf8",
);
const rightPanelSource = readFileSync(
  new URL("./features/workbench/RightPanel.tsx", import.meta.url),
  "utf8",
);

test("keeps the one-second clock out of the full activity timeline render", () => {
  const timelineRender = timelineSource.slice(
    timelineSource.indexOf("export const TurnActivityTimeline"),
    timelineSource.indexOf("function TurnTimingText"),
  );

  assert.match(timelineRender, /memo\(function TurnActivityTimeline/);
  assert.match(timelineRender, /<TurnTimingText/);
  assert.doesNotMatch(timelineRender, /useTimelineClock\(/);
});

test("memoizes tool activity parsing by stable call and result objects", () => {
  assert.match(toolCardSource, /memo\(function ToolActivityCard/);
  assert.match(
    toolCardSource,
    /useMemo\(\(\) => buildToolActivity\(call, result\), \[call, result\]\)/,
  );
});

test("does not force a synchronous scroll layout for every event batch", () => {
  assert.doesNotMatch(
    messageListSource,
    /\[events, messages, renderedMessageCount, updateScrollToEndVisibility\]/,
  );
  assert.match(messageListSource, /new ResizeObserver/);
  assert.match(messageListSource, /window\.requestAnimationFrame/);
});

test("keeps switching feedback visible and consistent for cold and cached loads", () => {
  assert.match(loadingIndicatorSource, /<ShimmerText/);
  assert.match(
    conversationHeaderSource,
    /<ConversationLoadingIndicator label="正在加载会话内容"/,
  );
  assert.match(
    messageListSource,
    /<ConversationLoadingIndicator label="正在同步最新内容"/,
  );

  const connectSource = conversationControllerSource.slice(
    conversationControllerSource.indexOf("private connect(): void"),
    conversationControllerSource.indexOf("private disconnect(): void"),
  );
  assert.match(connectSource, /const loadingFeedbackPainted/);
  assert.ok(
    connectSource.indexOf("await loadingFeedbackPainted") <
      connectSource.indexOf('type: "syncCompleted"'),
  );
});

test("keeps unchanged sidebar rows behind a memo boundary", () => {
  assert.match(sidebarRowSource, /memo\(function SidebarThreadRow/);
});

test("schedules task navigation as interruptible rendering work", () => {
  const selectThreadSource = appSource.slice(
    appSource.indexOf("function selectThread"),
    appSource.indexOf("useEffect", appSource.indexOf("function selectThread")),
  );

  assert.match(selectThreadSource, /startTransition\(\(\) => \{/);
  assert.match(selectThreadSource, /setActiveThreadId\(threadId\)/);
});

test("limits commit tracing to the newly appended event tail", () => {
  assert.doesNotMatch(appSource, /new Set\(events\.map/);
  assert.match(appSource, /oldestPendingSeq/);
  assert.match(appSource, /event\.seq < oldestPendingSeq/);
});

test("tracks task lifecycle events at the conversation registry boundary", () => {
  assert.match(
    appSource,
    /conversationRegistry\?\.subscribeToEvents\(forwardConversationEvent\)/,
  );
  assert.match(appSource, /useVisibleThreadActivityRead\(/);
  assert.match(activityHookSource, /store\.subscribeToChanges/);
  assert.match(activityHookSource, /markVisibleThreadActivityRead\(/);
  assert.match(conversationControllerSource, /openThreadActivityStream\(/);
  assert.match(
    conversationControllerSource,
    /activityStore\.applyEvent\(event\)/,
  );
  assert.doesNotMatch(
    conversationControllerSource,
    /activityRetentionReleases/,
  );
  assert.match(activityStoreSource, /incomingTurnId !== current\.turnId/);
});

test("does not poll every processing task on a timer", () => {
  assert.doesNotMatch(appSource, /refreshProcessingStatuses/);
  assert.doesNotMatch(appSource, /setInterval\(refreshProcessingStatuses/);
});

test("subscribes each sidebar row to its own activity status", () => {
  assert.match(
    sidebarRowSource,
    /useThreadActivityStatus\(activityStore, thread\.id\)/,
  );
  assert.doesNotMatch(appSource, /threadActivityStatuses/);
});

test("keeps the application shell off the full live-event subscription", () => {
  assert.match(appSource, /useConversationSessionSelector\(/);
  assert.doesNotMatch(appSource, /useConversationSession\(/);
  assert.match(conversationHookSource, /isEqual\(cached\.selected, selected\)/);
});

test("subscribes event-heavy surfaces at their own render boundaries", () => {
  assert.match(liveMessageListSource, /useConversationSession\(/);
  assert.match(liveComposerSource, /useConversationSession\(/);
  assert.match(liveComposerSource, /useDeferredValue\(events\)/);
  assert.match(rightPanelSource, /useConversationSession\(/);
});

test("memoizes the composer during urgent tool-event passes", () => {
  assert.match(composerSource, /const MemoizedComposer = memo\(/);
  assert.match(liveComposerSource, /<Composer/);
});

test("keeps the submitted composer draft until the server accepts it", () => {
  const submitDraft = composerSource.slice(
    composerSource.indexOf("const submitDraft = async"),
    composerSource.indexOf("function executeComposerEnterCommand"),
  );
  const acceptedAt = submitDraft.indexOf("const accepted = await onSubmit");
  const rejectedAt = submitDraft.indexOf("if (!accepted) return");
  const clearedAt = submitDraft.indexOf('onChange("")');
  assert.ok(acceptedAt >= 0);
  assert.ok(rejectedAt > acceptedAt);
  assert.ok(clearedAt > rejectedAt);
});
