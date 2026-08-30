import {
  startTransition,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type CSSProperties,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
import {
  Activity,
  Bot,
  Cable,
  Inbox,
  LayoutDashboard,
  Library,
  ShieldCheck,
  Workflow,
  X,
} from "lucide-react";
import { ApiClient } from "./api/client";
import type { StreamHandle } from "./api/client";
import type {
  DiffReviewFileContent,
  DiffReviewGitAction,
} from "./components/DiffReviewPanel";
import { LogViewer } from "./components/LogViewer";
import { ApprovalDialog } from "./components/ApprovalDialog";
import { PlanChoiceCard } from "./components/PlanChoiceCard";
import type { ImagePreviewSource } from "./components/PreviewHost";
import { ConnectionSidebarCollection } from "./components/connections";
import {
  EnterpriseSidebarCollection,
  FlowEnterpriseWorkspace,
} from "./components/enterprise";
import {
  FlowWorkspaceProvider,
  FlowWorkspaceTitle,
} from "./components/enterprise/flowAgentSelection";
import type { EnterprisePageHeader } from "./components/enterprise/pageHeader";
import {
  SettingsPanel as RedesignedSettingsPanel,
  type SettingsTab,
} from "./components/SettingsPanel";
import { TaskSearchDialog } from "./components/TaskSearchDialog";
import {
  resolveActiveFlowPrimaryView,
  resolveSidebarDestination,
  type FlowPrimaryView,
} from "./workspaceNavigation";
import {
  resolveFlowLibraryProvider,
  updateFlowLibraryBindings,
} from "./flowLibraryBinding";
import {
  newTaskComposerDraftKey,
  threadComposerDraftKey,
} from "./composerDrafts";
import { useComposerDraft } from "./useComposerDraft";
import {
  conversationStreamEventTrace,
  rendererTraceTime,
  type ConversationStreamEventTrace,
} from "./conversationRenderTrace";

import { closeToolTabState } from "./toolTabState";
import { resolveMarkdownLink } from "./markdownLinks";
import {
  useWorkspacePathIndex,
  WorkspacePathIndexContext,
} from "./components/WorkspacePathProvider";
import {
  closeAppWindow,
  deleteProviderApiKey,
  ensureLibraryProviderService,
  getDroppedContextFiles,
  getBackendStartupStatus,
  getRecentWorkspaces,
  listSecretSources,
  loadPlatformInfo,
  newAppWindow,
  openExternal,
  onBackendStartupStatus,
  openPath,
  quitApp,
  recordConversationRenderTrace,
  selectContextFiles,
  selectPluginDirectory,
  selectWorkspace,
  setProviderApiKey,
  showSystemNotification,
} from "./platform";
import {
  formatTaskCompletionNotificationBody,
  formatTaskCompletionNotificationTitle,
  playCompletionChime,
  readTaskNotificationPreferences,
  resolveTaskCompletionNotificationContent,
  shouldDeliverTaskNotification,
  writeTaskNotificationPreferences,
} from "./taskNotifications";
import { ThreadActivityStore } from "./threadActivityStore";
import { promoteThreadByActivity } from "./threadRecency";
import { errorMessage, isAbortError } from "./errorMessage";
import { threadTitleFromPrompt } from "./threadTitle";
import { workspaceRootKey } from "./workspaceRootKey";
import { isSpreadsheetFilePath } from "./spreadsheetFormats.ts";
import { shouldPromptForWindowsSandboxSetup } from "./windowsSandboxSetup";
import {
  applyAppearance,
  readAppearanceSettings,
  resolveTheme,
  watchSystemTheme,
  writeAppearanceSettings,
  type ResolvedTheme,
} from "./appearance";
import {
  readPersonalizationSettings,
  writePersonalizationSettings,
} from "./personalization";
import { canCancelTurn } from "./threadRunState";
import {
  readEditorPreferences,
  writeEditorPreferences,
} from "./editorPreferences";
import {
  readDraftModelSelection,
  readLastActiveThreadId,
  readSidebarNavigationState,
  resolveDraftModelSelection,
  updateSidebarNavigationState,
  writeDraftModelSelection,
  writeLastActiveThreadId,
} from "./workbenchPreferences";
import type {
  AgentEvent,
  AppSettings,
  BackendStartupStatus,
  ArtifactContent,
  ArtifactDescriptor,
  BrowserNewTabRequest,
  BrowserNavigationRequest,
  CollaborationMode,
  CodexAccountStatus,
  CodexLoginStart,
  ContextSourceFile,
  ContextStatus,
  ExperienceMode,
  GoalSnapshot,
  GoalStatus,
  InlineImageAttachment,
  InlineMessageContentPart,
  LibraryProviderId,
  McpServerInput,
  McpServerView,
  Message,
  PlatformInfo,
  PluginView,
  Project,
  PermissionMode,
  ProviderHealth,
  ProviderHealthCheckResult,
  ProviderKind,
  ProviderModelSyncResult,
  ProviderSecretOutcome,
  ProviderSettings,
  PreviewTarget,
  RecentWorkspace,
  SandboxDescriptor,
  SecretSources,
  SkillDescriptor,
  AgentListItem,
  TerminalEvent,
  TerminalSession,
  Thread,
  ThreadMcpServerView,
  ThreadModelSelection,
  TurnFileChange,
  UserInputResponse,
  WorkspaceDiff,
  WorkspaceDiffHunk,
  WorkspaceDiffHunkAction,
  WorkspaceEntry,
  WorkspaceFilePreview,
  WorkspaceTree,
  WebPreviewState,
  WindowsSandboxSetupStatus,
} from "./types";
import { reuseUnchangedAgentList } from "./agentListState";
import { PreviewSessionStore } from "./previewSessionStore";
import { ConversationSessionRegistry } from "./conversationSessionController";
import {
  browserSessionId,
  initializeBrowserTabSession,
  newBrowserTabSessionId,
} from "./browserNavigation";
import { useConversationSessionSelector } from "./useConversationSession";
import {
  useThreadRunState,
  useVisibleThreadActivityRead,
} from "./useThreadActivityStore";
import {
  appConversationStateEqual,
  selectAppConversationState,
} from "./appConversationState";
import { Sidebar } from "./features/sidebar/Sidebar";
import {
  browserTabTitle,
  toolTabTitle,
  type ToolTab,
  type ToolTabKind,
} from "./toolTabs";
import {
  artifactReferencesFromText,
  type ArtifactReference,
} from "./artifactReferences";
import { friendlyProviderError } from "./providerErrors";
import {
  ConversationLoadErrorState,
  ConversationLoadingState,
  GoalStrip,
  ThreadHeader,
} from "./features/conversation/ConversationHeader";
import { LiveConversationMessageList } from "./features/conversation/LiveConversationMessageList";
import { TopBar } from "./features/shell/TopBar";
import {
  clampPanelSize,
  readWorkspaceLayoutPreferences,
  resolveWorkspaceLayout,
  workspaceLayoutStorageKey,
  type WorkspaceLayout,
  type WorkspaceLayoutPreferences,
  type WorkspaceRightPanelKind,
} from "./features/shell/workspaceLayout";
import {
  AboutDialog,
  KeyboardShortcutsDialog,
  RenameDialog,
  TurnUndoDialog,
  WindowsSandboxSetupDialog,
  type RenameTarget,
  type TurnUndoDialogState,
} from "./features/shell/AppDialogs";
import {
  ConversationFileDropTarget,
  useConversationFileDrop,
  type ComposerFileDropHandle,
  type NewTaskLaunchMode,
} from "./features/composer/Composer";
import { LiveConversationComposer } from "./features/composer/LiveConversationComposer";
import { workspaceName } from "./workspaceName";
import { RightPanel } from "./features/workbench/RightPanel";
import {
  NewTaskState,
  OfflineState,
} from "./features/conversation/ConversationEmptyStates";

type ServerStatus = "checking" | "online" | "offline";

const emptyConversationMessages: Message[] = [];
const emptyConversationEvents: AgentEvent[] = [];

type DirectToolCommand =
  { kind: "run"; command: string } | { kind: "read"; path: string };

type WorkspaceResizeSide = "left" | "right";

type WorkspaceResizeDrag = {
  side: WorkspaceResizeSide;
  preferenceKey: keyof WorkspaceLayoutPreferences;
  pointerId: number;
  startX: number;
  startSize: number;
  latestSize: number;
  min: number;
  max: number;
};

const experienceModeStorageKey = "opentopia.experience-mode.v1";
const collaborationModeStorageKey = "opentopia.collaboration-mode.v1";
const flowLibraryBindingsStorageKey = "opentopia.flow-library-bindings.v1";
const contextRailInlineMinWidth = 1120;

function readExperienceMode(): ExperienceMode {
  if (typeof window === "undefined") return "code";
  try {
    const stored = window.localStorage.getItem(experienceModeStorageKey);
    return stored === "work" || stored === "flow" ? stored : "code";
  } catch {
    return "code";
  }
}

function readCollaborationMode(): CollaborationMode {
  if (typeof window === "undefined") return "default";
  try {
    const value = window.localStorage.getItem(collaborationModeStorageKey);
    return value === "plan" || value === "goal" ? value : "default";
  } catch {
    return "default";
  }
}

function readFlowLibraryBindings(): Record<string, LibraryProviderId> {
  if (typeof window === "undefined") return {};
  try {
    const parsed = JSON.parse(
      window.localStorage.getItem(flowLibraryBindingsStorageKey) ?? "{}",
    ) as Record<string, unknown>;
    return Object.fromEntries(
      Object.entries(parsed).filter(
        (entry): entry is [string, LibraryProviderId] =>
          entry[1] === "sag" || entry[1] === "graph-rag",
      ),
    );
  } catch {
    return {};
  }
}

function reusableGoalId(
  mode: CollaborationMode,
  snapshot: GoalSnapshot | null,
): string | undefined {
  if (!snapshot || mode !== "goal") return undefined;
  if (["completed", "cancelled"].includes(snapshot.workForm.status)) {
    return undefined;
  }
  return snapshot.goal.id;
}

function emptyContextUsage(): ContextStatus["usage"] {
  return {
    modelRequests: 0,
    agentModelRequests: 0,
    compactionModelRequests: 0,
    auxiliaryModelRequests: 0,
    providerResponses: 0,
    providerUsageCoverage: null,
    inputTokens: 0,
    outputTokens: 0,
    totalTokens: 0,
    uncachedInputTokens: 0,
    cachedInputTokens: 0,
    cacheWriteTokens: 0,
    reasoningTokens: 0,
    localInputEstimate: 0,
    rawInputEstimate: 0,
    estimateCalibrationFactor: null,
    estimateErrorMean: null,
    estimateErrorP95: null,
    rawEstimateErrorMean: null,
    rawEstimateErrorP95: null,
    compactions: 0,
    nativeCompactions: 0,
    providerFallbacks: 0,
    warnings: 0,
    compactionInputTokens: 0,
    checkpointTokens: 0,
    compactionLatencyMs: 0,
    lastFactRetentionPercent: 0,
    lastActiveConstraintRetentionPercent: 0,
  };
}

export function App() {
  const [platform, setPlatform] = useState<PlatformInfo | null>(null);
  const [client, setClient] = useState<ApiClient | null>(null);
  const threadActivityStore = useMemo(() => new ThreadActivityStore(), []);
  const conversationRegistry = useMemo(
    () =>
      client
        ? new ConversationSessionRegistry(client, 8, threadActivityStore)
        : null,
    [client, threadActivityStore],
  );
  useEffect(
    () => () => {
      conversationRegistry?.dispose();
    },
    [conversationRegistry],
  );
  const [serverStatus, setServerStatus] = useState<ServerStatus>("checking");
  const [serverError, setServerError] = useState<string | null>(null);
  const [bootstrapRetryNonce, setBootstrapRetryNonce] = useState(0);
  const [serverProbing, setServerProbing] = useState(true);
  const [backendStartupStatus, setBackendStartupStatus] =
    useState<BackendStartupStatus | null>(null);
  const clientEndpointRef = useRef<string | null>(null);
  const readyStartupBootstrapRef = useRef<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);

  useEffect(() => {
    const applyStatus = (nextStatus: BackendStartupStatus) => {
      setBackendStartupStatus((currentStatus) =>
        !currentStatus ||
        Date.parse(nextStatus.updatedAt) >= Date.parse(currentStatus.updatedAt)
          ? nextStatus
          : currentStatus,
      );
    };
    const unsubscribe = onBackendStartupStatus(applyStatus);
    void getBackendStartupStatus()
      .then((status) => {
        if (status) applyStatus(status);
      })
      .catch(() => undefined);
    return unsubscribe;
  }, []);
  const [projects, setProjects] = useState<Project[]>([]);
  const [threads, setThreads] = useState<Thread[]>([]);
  const [activeThreadId, setActiveThreadId] = useState<string | null>(null);
  useVisibleThreadActivityRead(threadActivityStore, activeThreadId);
  useEffect(
    () =>
      threadActivityStore.subscribeToChanges((threadId, activity) => {
        if (!activity?.optimistic) return;
        setThreads((current) =>
          promoteThreadByActivity(current, threadId, activity.updatedAt),
        );
      }),
    [threadActivityStore],
  );
  const conversationEventEffectRef = useRef<(event: AgentEvent) => void>(
    () => {},
  );
  const forwardConversationEvent = useCallback((event: AgentEvent) => {
    conversationEventEffectRef.current(event);
  }, []);
  useEffect(
    () => conversationRegistry?.subscribeToEvents(forwardConversationEvent),
    [conversationRegistry, forwardConversationEvent],
  );
  const {
    controller: activeConversationController,
    state: activeConversationState,
  } = useConversationSessionSelector(
    conversationRegistry,
    activeThreadId,
    selectAppConversationState,
    appConversationStateEqual,
  );
  const activeThreadRunState = useThreadRunState(
    threadActivityStore,
    activeThreadId,
  );
  const [draftProjectId, setDraftProjectId] = useState<string | null>(null);
  // Model picked on the new-task screen, before a thread exists to pin it to.
  // Carried into the thread the draft creates.
  const [draftModelSelection, setDraftModelSelection] =
    useState<ThreadModelSelection | null>(readDraftModelSelection);
  const [experienceMode, setExperienceMode] =
    useState<ExperienceMode>(readExperienceMode);
  const [flowPrimaryView, setFlowPrimaryView] =
    useState<FlowPrimaryView>("conversation");
  const [flowPageHeader, setFlowPageHeader] =
    useState<EnterprisePageHeader | null>(null);

  function navigateFlowPrimaryView(view: FlowPrimaryView) {
    setFlowPageHeader(null);
    setFlowPrimaryView(view);
    if (view !== "conversation") {
      setToolStageOpen(false);
      setActiveToolTabId(null);
    }
    setConversationCollapsed(false);
  }
  const [flowLibraryBindings, setFlowLibraryBindings] = useState<
    Record<string, LibraryProviderId>
  >(readFlowLibraryBindings);
  const [draftFlowLibraryProvider, setDraftFlowLibraryProvider] =
    useState<LibraryProviderId | null>(null);
  const [collaborationMode, setCollaborationMode] = useState<CollaborationMode>(
    readCollaborationMode,
  );
  const [goalSnapshot, setGoalSnapshot] = useState<GoalSnapshot | null>(null);
  const [goalAction, setGoalAction] = useState<GoalStatus | "run" | null>(null);
  const [selectedWorkspaceRoot, setSelectedWorkspaceRoot] = useState<
    string | null
  >(null);
  const [workspaceError, setWorkspaceError] = useState<string | null>(null);
  const [isPickingWorkspace, setIsPickingWorkspace] = useState(false);
  const conversationLoadState = activeConversationState?.loadState ?? {
    threadId: null,
    status: "idle" as const,
    error: null,
  };
  const [agentItems, setAgentItems] = useState<AgentListItem[]>([]);
  const [terminalEvents, setTerminalEvents] = useState<TerminalEvent[]>([]);
  const [terminalSession, setTerminalSession] =
    useState<TerminalSession | null>(null);
  const composerDraftKey = activeThreadId
    ? threadComposerDraftKey(activeThreadId)
    : newTaskComposerDraftKey(experienceMode, draftProjectId);
  const {
    text: composer,
    contextSources,
    selectedSkillIds,
    setText: setComposer,
    setContextSources,
    setSelectedSkillIds,
  } = useComposerDraft(composerDraftKey);
  const conversationComposerFileDropHandle =
    useRef<ComposerFileDropHandle>(null);
  const conversationFileDrop = useConversationFileDrop(
    conversationComposerFileDropHandle,
  );
  const [newTaskLaunchMode, setNewTaskLaunchMode] =
    useState<NewTaskLaunchMode>("local");
  const [skills, setSkills] = useState<SkillDescriptor[]>([]);
  const [skillsRevision, setSkillsRevision] = useState(0);
  const [isCreatingThread, setIsCreatingThread] = useState(false);
  const activeTurnId = activeThreadRunState.activeTurnId;
  const queuedMessageCount = activeConversationState?.queuedMessageCount ?? 0;
  const pendingApprovalIds = activeConversationState?.pendingApprovalIds ?? [];
  const decidingApprovalId =
    activeConversationState?.decidingApprovalId ?? null;
  const approvalDecisionError = activeConversationState?.approvalError ?? null;
  const pendingUserInput = activeConversationState?.pendingUserInput ?? [];
  const submittingUserInputId =
    activeConversationState?.submittingUserInputId ?? null;
  const userInputError = activeConversationState?.userInputError ?? null;
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settingsInitialTab, setSettingsInitialTab] =
    useState<SettingsTab>("general");
  const openSettings = useCallback(() => {
    setSettingsInitialTab("general");
    setSettingsOpen(true);
  }, []);
  const openModelSettings = useCallback(() => {
    setSettingsInitialTab("providers");
    setSettingsOpen(true);
  }, []);
  const openPermissionSettings = useCallback(() => {
    setSettingsInitialTab("permissions");
    setSettingsOpen(true);
  }, []);
  const closeSettings = useCallback(() => {
    setSettingsOpen(false);
    setSettingsInitialTab("general");
  }, []);
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [taskNotificationPreferences, setTaskNotificationPreferences] =
    useState(readTaskNotificationPreferences);
  const [appearance, setAppearance] = useState(readAppearanceSettings);
  const [resolvedTheme, setResolvedTheme] = useState<ResolvedTheme>(() =>
    resolveTheme(readAppearanceSettings().mode),
  );
  const [personalization, setPersonalization] = useState(
    readPersonalizationSettings,
  );
  const [editorPreferences, setEditorPreferences] = useState(
    readEditorPreferences,
  );
  const [providerHealth, setProviderHealth] = useState<ProviderHealth[]>([]);
  const [codexAccount, setCodexAccount] = useState<CodexAccountStatus | null>(
    null,
  );
  const [codexAccountLoading, setCodexAccountLoading] = useState(false);
  const [codexAccountError, setCodexAccountError] = useState<string | null>(
    null,
  );
  const refreshCodexAccount = useCallback(async () => {
    if (!client) return;
    setCodexAccountLoading(true);
    try {
      setCodexAccount(await client.getCodexAccount());
      setCodexAccountError(null);
    } catch (error) {
      setCodexAccountError(errorMessage(error));
    } finally {
      setCodexAccountLoading(false);
    }
  }, [client]);
  const [windowsSandboxSetup, setWindowsSandboxSetup] =
    useState<WindowsSandboxSetupStatus | null>(null);
  const [windowsSandboxSetupBusy, setWindowsSandboxSetupBusy] = useState(false);
  const [windowsSandboxSetupError, setWindowsSandboxSetupError] = useState<
    string | null
  >(null);
  const [windowsSandboxPromptDismissed, setWindowsSandboxPromptDismissed] =
    useState(false);
  const isWindows = platform?.os === "win32" || platform?.os === "windows";

  useEffect(() => {
    if (!client || !isWindows || serverStatus !== "online") {
      setWindowsSandboxSetup(null);
      setWindowsSandboxSetupError(null);
      return;
    }
    let cancelled = false;
    setWindowsSandboxSetupBusy(true);
    void client
      .getWindowsSandboxSetup()
      .then((status) => {
        if (cancelled) return;
        setWindowsSandboxSetup(status);
        setWindowsSandboxSetupError(null);
      })
      .catch((error: unknown) => {
        if (!cancelled) setWindowsSandboxSetupError(errorMessage(error));
      })
      .finally(() => {
        if (!cancelled) setWindowsSandboxSetupBusy(false);
      });
    return () => {
      cancelled = true;
    };
  }, [client, isWindows, serverStatus]);

  const setupWindowsSandbox =
    useCallback(async (): Promise<WindowsSandboxSetupStatus> => {
      if (!client) throw new Error("后端尚未连接");
      setWindowsSandboxSetupBusy(true);
      setWindowsSandboxSetupError(null);
      try {
        const status = await client.setupWindowsSandbox();
        setWindowsSandboxSetup(status);
        return status;
      } catch (error) {
        setWindowsSandboxSetupError(errorMessage(error));
        // Setup is deliberately repairable and can leave useful partial state.
        // Refresh after a failed privileged operation so the persistent
        // settings entry reflects what actually exists and offers the right
        // recovery action.
        try {
          setWindowsSandboxSetup(await client.getWindowsSandboxSetup());
        } catch {
          // Preserve the original setup error; it is the actionable failure.
        }
        throw error;
      } finally {
        setWindowsSandboxSetupBusy(false);
      }
    }, [client]);
  const removeWindowsSandbox =
    useCallback(async (): Promise<WindowsSandboxSetupStatus> => {
      if (!client) throw new Error("后端尚未连接");
      setWindowsSandboxSetupBusy(true);
      setWindowsSandboxSetupError(null);
      try {
        const status = await client.removeWindowsSandbox();
        setWindowsSandboxSetup(status);
        setWindowsSandboxPromptDismissed(false);
        return status;
      } catch (error) {
        setWindowsSandboxSetupError(errorMessage(error));
        throw error;
      } finally {
        setWindowsSandboxSetupBusy(false);
      }
    }, [client]);
  const [providerTest, setProviderTest] = useState<{
    providerId: string;
    status: "testing" | "complete";
    result?: ProviderHealthCheckResult;
  } | null>(null);
  const [secretSources, setSecretSources] = useState<SecretSources | null>(
    null,
  );
  const [isSavingSecret, setIsSavingSecret] = useState(false);
  const [logViewerOpen, setLogViewerOpen] = useState(false);
  const [isSavingSettings, setIsSavingSettings] = useState(false);
  const [workspaceTree, setWorkspaceTree] = useState<WorkspaceTree | null>(
    null,
  );
  const [filePreview, setFilePreview] = useState<WorkspaceFilePreview | null>(
    null,
  );
  const [workspaceDiff, setWorkspaceDiff] = useState<WorkspaceDiff | null>(
    null,
  );
  const [sandbox, setSandbox] = useState<SandboxDescriptor | null>(null);
  const [mcpServers, setMcpServers] = useState<McpServerView[]>([]);
  const [plugins, setPlugins] = useState<PluginView[]>([]);
  const [threadMcpServers, setThreadMcpServers] = useState<
    ThreadMcpServerView[]
  >([]);
  const [workbenchError, setWorkbenchError] = useState<string | null>(null);
  const [isRefreshingWorkbench, setIsRefreshingWorkbench] = useState(false);
  const workbenchRefreshControllerRef = useRef<AbortController | null>(null);
  const [artifacts, setArtifacts] = useState<ArtifactDescriptor[]>([]);
  const [contextStatus, setContextStatus] = useState<ContextStatus | null>(
    null,
  );
  const [isCompactingContext, setIsCompactingContext] = useState(false);
  const [revertingDiffPath, setRevertingDiffPath] = useState<string | null>(
    null,
  );
  const [hunkActionKey, setHunkActionKey] = useState<string | null>(null);
  // File the review panel was last asked to show. Bumped with a nonce so
  // re-picking the same file after browsing away still refocuses it.
  const [reviewFileRequest, setReviewFileRequest] = useState<{
    path: string;
    nonce: number;
  } | null>(null);
  const [toolTabs, setToolTabs] = useState<ToolTab[]>([]);
  const toolTabsRef = useRef<ToolTab[]>(toolTabs);
  toolTabsRef.current = toolTabs;
  const [previewSessionStore] = useState(() => new PreviewSessionStore());
  const hasDirtyPreviewSessions = useSyncExternalStore(
    previewSessionStore.subscribeToDirtySessions,
    previewSessionStore.hasDirtySessions,
    previewSessionStore.hasDirtySessions,
  );
  const [activeToolTabId, setActiveToolTabId] = useState<string | null>(null);
  const [toolStageOpen, setToolStageOpen] = useState(false);
  const [conversationCollapsed, setConversationCollapsed] = useState(false);
  const [contextRailOpen, setContextRailOpen] = useState(false);
  const [contextRailCollapsed, setContextRailCollapsed] = useState(false);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(
    () => readSidebarNavigationState().collapsed,
  );
  const [taskSearchOpen, setTaskSearchOpen] = useState(false);
  const [keyboardShortcutsOpen, setKeyboardShortcutsOpen] = useState(false);

  useEffect(() => {
    if (!hasDirtyPreviewSessions) return;
    const confirmUnload = (event: BeforeUnloadEvent) => {
      event.preventDefault();
    };
    window.addEventListener("beforeunload", confirmUnload);
    return () => window.removeEventListener("beforeunload", confirmUnload);
  }, [hasDirtyPreviewSessions]);
  const [aboutOpen, setAboutOpen] = useState(false);
  const [renameTarget, setRenameTarget] = useState<RenameTarget | null>(null);
  const [turnUndoDialog, setTurnUndoDialog] =
    useState<TurnUndoDialogState | null>(null);
  const [workspaceLayoutPreferences, setWorkspaceLayoutPreferences] =
    useState<WorkspaceLayoutPreferences>(readWorkspaceLayoutPreferences);
  const [workspaceWidth, setWorkspaceWidth] = useState(() =>
    typeof window === "undefined" ? 1440 : window.innerWidth,
  );
  const [workspaceResizeSide, setWorkspaceResizeSide] =
    useState<WorkspaceResizeSide | null>(null);
  const workspaceRef = useRef<HTMLElement>(null);
  const workspaceResizeDragRef = useRef<WorkspaceResizeDrag | null>(null);
  const pendingWorkspaceSizeRef = useRef<{
    key: keyof WorkspaceLayoutPreferences;
    value: number;
  } | null>(null);
  const workspaceResizeFrameRef = useRef<number | null>(null);
  const markdownNavigationIdRef = useRef(0);
  const browserTabSequenceRef = useRef(0);
  const browserTabLaunchGenerationRef = useRef(0);
  const browserNewTabRequestHandlerRef = useRef<
    (request: BrowserNewTabRequest) => void
  >(() => {});
  const taskNotificationPreferencesRef = useRef(taskNotificationPreferences);
  const pendingTaskNotificationEventsRef = useRef<AgentEvent[]>([]);
  const pendingEventCommitTraceRef = useRef(
    new Map<
      string,
      {
        eventTrace: ConversationStreamEventTrace;
        eventSeq: number;
        receivedClockMs: number;
      }
    >(),
  );
  const activeThreadIdRef = useRef<string | null>(null);
  const agentRefreshRequestRef = useRef<(() => void) | null>(null);

  activeThreadIdRef.current = activeThreadId;

  useEffect(
    () => () => {
      browserTabLaunchGenerationRef.current += 1;
      for (const tab of toolTabsRef.current) releaseBrowserTabSession(tab);
    },
    [],
  );
  useEffect(() => {
    const subscribe = window.opentopia?.browserHost?.onNewTabRequested;
    if (!subscribe) return;
    const unsubscribe = subscribe((request) =>
      browserNewTabRequestHandlerRef.current(request),
    );
    return typeof unsubscribe === "function" ? unsubscribe : undefined;
  }, []);
  const markThreadActivityRead = useCallback(
    (threadId: string) => threadActivityStore.markRead(threadId),
    [threadActivityStore],
  );

  const activeThread = useMemo(
    () => threads.find((thread) => thread.id === activeThreadId) ?? null,
    [threads, activeThreadId],
  );
  const activeFlowLibraryProvider = resolveFlowLibraryProvider(
    activeThreadId,
    flowLibraryBindings,
    draftFlowLibraryProvider,
  );
  const changeFlowLibraryProvider = useCallback(
    (provider: LibraryProviderId | null) => {
      if (activeThreadId) {
        setFlowLibraryBindings((current) =>
          updateFlowLibraryBindings(current, activeThreadId, provider),
        );
      } else {
        setDraftFlowLibraryProvider(provider);
      }
      if (!provider) return;
      setActionError(null);
      void ensureLibraryProviderService(provider)
        .then((runtime) => {
          if (runtime?.state === "unavailable") {
            throw new Error(runtime.message || "资料库服务尚未就绪");
          }
        })
        .catch((error: unknown) => {
          setActionError(`无法启动资料库检索：${errorMessage(error)}`);
        });
    },
    [activeThreadId],
  );
  useEffect(() => {
    if (!activeThread) return;
    writeLastActiveThreadId(activeThread.experienceMode, activeThread.id);
  }, [activeThread]);
  const isSending = activeThreadId
    ? activeThreadRunState.sending
    : isCreatingThread;
  const isConversationReady =
    activeThreadId !== null &&
    conversationLoadState.threadId === activeThreadId &&
    conversationLoadState.status === "ready";
  const conversationLoadError =
    activeThreadId !== null &&
    conversationLoadState.threadId === activeThreadId &&
    conversationLoadState.status === "error"
      ? conversationLoadState.error
      : null;
  const isConversationLoading =
    activeThreadId !== null &&
    !isConversationReady &&
    conversationLoadError === null;
  const conversationGoalSnapshot = isConversationReady ? goalSnapshot : null;
  const conversationActiveTurnId = isConversationReady ? activeTurnId : null;
  const conversationTurnCanBeCancelled = canCancelTurn(activeThreadRunState);
  const conversationTurnIsCancelling = activeThreadRunState.cancelling;
  const draftProject = useMemo(
    () => projects.find((project) => project.id === draftProjectId) ?? null,
    [draftProjectId, projects],
  );
  const activeProject = useMemo(() => {
    const projectId = activeThread?.projectId ?? draftProjectId;
    return projects.find((project) => project.id === projectId) ?? null;
  }, [activeThread?.projectId, draftProjectId, projects]);
  const currentWorkspaceRoot =
    selectedWorkspaceRoot ??
    activeThread?.workspaceRoot ??
    draftProject?.workspaceRoot ??
    null;
  const workspacePathIndex = useWorkspacePathIndex({
    client,
    threadId: activeThread?.id ?? null,
    workspaceRoot: activeThread?.workspaceRoot ?? null,
  });
  const activeToolTab = useMemo(
    () => toolTabs.find((tab) => tab.id === activeToolTabId) ?? null,
    [activeToolTabId, toolTabs],
  );
  const flowPrimarySurface =
    experienceMode === "flow" && flowPrimaryView !== "conversation";
  const flowInspectorOpen =
    flowPrimarySurface && flowPrimaryView !== "knowledge";
  const workspaceRightPanelKind: WorkspaceRightPanelKind = flowInspectorOpen
    ? "inspector"
    : toolStageOpen
      ? "tool"
      : "context";
  const rightResizePreferenceKey: keyof WorkspaceLayoutPreferences =
    flowInspectorOpen ? "inspectorRight" : "toolRight";
  const sidebarDestination = resolveSidebarDestination({
    experienceMode,
    flowPrimaryView,
    toolStageOpen,
    activeToolKind: activeToolTab?.kind ?? null,
  });
  const activeToolRequiresFullWorkspace = activeToolTab?.kind === "extensions";
  const toolStageCoversConversation =
    toolStageOpen && (conversationCollapsed || activeToolRequiresFullWorkspace);
  const terminalToolActive =
    toolStageOpen && activeToolTab?.kind === "terminal";
  const pendingApprovalQueue =
    activeConversationState?.pendingApprovalQueue ?? [];
  const activeApproval = pendingApprovalQueue[0]?.payload ?? null;
  const activeUserInput = isConversationReady
    ? (pendingUserInput[0] ?? null)
    : null;
  useEffect(() => {
    setTurnUndoDialog(null);
    pendingEventCommitTraceRef.current.clear();
    pendingTaskNotificationEventsRef.current = [];
  }, [activeThreadId]);

  useEffect(() => {
    if (!activeApproval) return;
    setConversationCollapsed(false);
    setToolStageOpen(false);
    setActionError((current) =>
      current &&
      /resolve the pending approval before starting another turn/i.test(current)
        ? null
        : current,
    );
  }, [activeApproval?.approval_id]);

  useEffect(() => {
    if (!activeUserInput) return;
    setConversationCollapsed(false);
    setToolStageOpen(false);
    setActionError((current) =>
      current &&
      /answer or dismiss the pending user decision before starting another turn/i.test(
        current,
      )
        ? null
        : current,
    );
  }, [activeUserInput?.request.requestId]);

  const workspaceLayout = useMemo(
    () =>
      resolveWorkspaceLayout(
        workspaceLayoutPreferences,
        workspaceWidth,
        workspaceRightPanelKind,
        toolStageCoversConversation,
      ),
    [
      toolStageCoversConversation,
      workspaceRightPanelKind,
      workspaceLayoutPreferences,
      workspaceWidth,
    ],
  );
  const contextRailAutoVisible =
    !flowPrimarySurface &&
    !toolStageOpen &&
    workspaceWidth - (sidebarCollapsed ? 0 : workspaceLayout.left) >=
      contextRailInlineMinWidth;
  const contextRailVisible =
    !flowPrimarySurface &&
    !toolStageOpen &&
    (contextRailOpen || (contextRailAutoVisible && !contextRailCollapsed));
  const workspaceStyle = {
    "--workspace-left-width": `${
      sidebarCollapsed ? 0 : workspaceLayout.left
    }px`,
    "--workspace-right-width": `${workspaceLayout.right}px`,
  } as CSSProperties;

  useEffect(() => {
    if (!contextRailVisible) return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      setContextRailOpen(false);
      if (contextRailAutoVisible) setContextRailCollapsed(true);
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [contextRailAutoVisible, contextRailVisible]);

  useEffect(() => {
    setContextRailOpen(false);
    setContextRailCollapsed(false);
  }, [activeThreadId]);

  useEffect(() => {
    const element = workspaceRef.current;
    if (!element) return;
    const updateWidth = () => {
      const nextWidth = Math.round(element.getBoundingClientRect().width);
      setWorkspaceWidth((current) =>
        current === nextWidth || nextWidth <= 0 ? current : nextWidth,
      );
    };
    const observer = new ResizeObserver(updateWidth);
    observer.observe(element);
    updateWidth();
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    try {
      window.localStorage.setItem(
        workspaceLayoutStorageKey,
        JSON.stringify(workspaceLayoutPreferences),
      );
    } catch {
      // Layout persistence is best-effort when storage is unavailable.
    }
  }, [workspaceLayoutPreferences]);

  useEffect(() => {
    try {
      window.localStorage.setItem(experienceModeStorageKey, experienceMode);
    } catch {
      // Mode persistence is best-effort when storage is unavailable.
    }
  }, [experienceMode]);

  useEffect(() => {
    try {
      window.localStorage.setItem(
        flowLibraryBindingsStorageKey,
        JSON.stringify(flowLibraryBindings),
      );
    } catch {
      // Test bindings are best-effort when browser storage is unavailable.
    }
  }, [flowLibraryBindings]);

  useEffect(() => {
    updateSidebarNavigationState({ collapsed: sidebarCollapsed });
  }, [sidebarCollapsed]);

  useEffect(() => {
    if (settings && !settings.enterprise.enabled && experienceMode === "flow") {
      setExperienceMode("code");
    }
  }, [experienceMode, settings]);

  useEffect(() => {
    taskNotificationPreferencesRef.current = taskNotificationPreferences;
    writeTaskNotificationPreferences(taskNotificationPreferences);
  }, [taskNotificationPreferences]);

  // Appearance is applied to <html> rather than held in React state, so the
  // whole tree (including portals and the Monaco host) picks it up from CSS.
  useEffect(() => {
    const resolved = applyAppearance(appearance);
    setResolvedTheme(resolved);
    writeAppearanceSettings(appearance);
    // The Windows caption buttons are outside the document, so CSS cannot
    // reach them; the main process has to repaint them for us.
    void window.opentopia?.setTheme(resolved);
  }, [appearance]);

  // Only follow the OS while the user is on "system"; an explicit choice must
  // survive the OS flipping underneath it.
  useEffect(() => {
    if (appearance.mode !== "system") return undefined;
    return watchSystemTheme(() => {
      const resolved = applyAppearance(appearance);
      setResolvedTheme(resolved);
      void window.opentopia?.setTheme(resolved);
    });
  }, [appearance]);

  useEffect(() => {
    writePersonalizationSettings(personalization);
  }, [personalization]);

  useEffect(() => {
    writeEditorPreferences(editorPreferences);
  }, [editorPreferences]);

  useEffect(() => {
    try {
      window.localStorage.setItem(
        collaborationModeStorageKey,
        collaborationMode,
      );
    } catch {
      // Mode persistence is best-effort when storage is unavailable.
    }
  }, [collaborationMode]);

  useEffect(
    () => () => {
      if (workspaceResizeFrameRef.current !== null) {
        window.cancelAnimationFrame(workspaceResizeFrameRef.current);
      }
    },
    [],
  );

  useEffect(() => {
    if (!settingsOpen) return;
    if (workspaceResizeFrameRef.current !== null) {
      window.cancelAnimationFrame(workspaceResizeFrameRef.current);
      workspaceResizeFrameRef.current = null;
    }
    pendingWorkspaceSizeRef.current = null;
    workspaceResizeDragRef.current = null;
    setWorkspaceResizeSide(null);
  }, [settingsOpen]);

  useEffect(() => {
    if (!client) return;
    let cancelled = false;
    void client
      .listSkills(currentWorkspaceRoot, activeThread?.id, experienceMode)
      .then((available) => {
        if (cancelled) return;
        setSkills(available);
        const ids = new Set(available.map((skill) => skill.id));
        setSelectedSkillIds((current) => current.filter((id) => ids.has(id)));
      })
      .catch(() => {
        if (!cancelled) setSkills([]);
      });
    return () => {
      cancelled = true;
    };
  }, [
    activeThread?.id,
    client,
    currentWorkspaceRoot,
    experienceMode,
    skillsRevision,
  ]);

  useEffect(() => {
    if (!client) return;
    let cancelled = false;
    void client
      .listPlugins({
        workspaceRoot: currentWorkspaceRoot,
        threadId: activeThreadId,
      })
      .then((available) => {
        if (!cancelled) setPlugins(available);
      })
      .catch(() => {
        if (!cancelled) setPlugins([]);
      });
    return () => {
      cancelled = true;
    };
  }, [activeThreadId, client, currentWorkspaceRoot]);

  const deliverTaskCompletionNotification = useCallback(
    (content: { userMessage: string; reply: string }, force = false) => {
      const preferences = taskNotificationPreferencesRef.current;
      if (!preferences.enabled) return;
      if (
        !force &&
        !shouldDeliverTaskNotification(preferences, document.hasFocus())
      ) {
        return;
      }

      if (preferences.completionSound) playCompletionChime();
      if (preferences.systemNotification) {
        void showSystemNotification({
          title: formatTaskCompletionNotificationTitle(content),
          body: formatTaskCompletionNotificationBody(content),
          silent: true,
        }).catch(() => {
          // Notification delivery is best-effort and must not affect the turn.
        });
      }
    },
    [],
  );

  const ingestEvent = useCallback(
    (event: AgentEvent) => {
      const isActiveThread = event.threadId === activeThreadIdRef.current;
      if (!isActiveThread) return;

      if (
        event.payload.type === "turn_started" ||
        event.payload.type === "assistant_message"
      ) {
        pendingTaskNotificationEventsRef.current.push(event);
      }

      const eventTrace = conversationStreamEventTrace(event);
      if (eventTrace) {
        const traceTime = rendererTraceTime();
        pendingEventCommitTraceRef.current.set(event.id, {
          eventTrace,
          eventSeq: event.seq,
          receivedClockMs: traceTime.rendererClockMs,
        });
        recordConversationRenderTrace({
          stage: "received",
          ...eventTrace,
          ...traceTime,
          change: "append",
          textLength: eventTrace.text.length,
        });
      }

      if (event.payload.type === "goal_updated") {
        setGoalSnapshot(event.payload.snapshot);
      }

      if (event.payload.type === "approval_requested") {
        setConversationCollapsed(false);
      }

      if (
        event.payload.type === "tool_call_started" &&
        event.payload.call.name === "computer"
      ) {
        openToolTab("computer");
      }

      if (event.payload.type === "browser_handoff_required") {
        setConversationCollapsed(false);
      }

      if (event.payload.type === "user_input_requested") {
        setConversationCollapsed(false);
      }

      if (event.payload.type === "error") {
        setActionError(
          `Agent 请求失败：${friendlyProviderError(event.payload.message)}`,
        );
      }

      if (event.payload.type === "turn_started" && event.turnId) {
        agentRefreshRequestRef.current?.();
      }

      if (event.payload.type === "turn_finished") {
        const notificationSession = conversationRegistry
          ?.get(event.threadId)
          .getSnapshot();
        deliverTaskCompletionNotification(
          resolveTaskCompletionNotificationContent(
            notificationSession?.messages ?? emptyConversationMessages,
            [
              ...(notificationSession?.events ?? emptyConversationEvents),
              ...pendingTaskNotificationEventsRef.current,
            ],
            event,
          ),
        );
        pendingTaskNotificationEventsRef.current =
          event.turnId === null || event.turnId === undefined
            ? []
            : pendingTaskNotificationEventsRef.current.filter(
                (pendingEvent) => pendingEvent.turnId !== event.turnId,
              );
      } else if (event.payload.type === "turn_cancelled") {
        pendingTaskNotificationEventsRef.current =
          event.turnId === null || event.turnId === undefined
            ? []
            : pendingTaskNotificationEventsRef.current.filter(
                (pendingEvent) => pendingEvent.turnId !== event.turnId,
              );
      }

      if (event.payload.type === "tool_call_finished") {
        if (
          isRecord(event.payload.result.metadata) &&
          event.payload.result.metadata.toolName === "create_skill" &&
          event.payload.result.metadata.success !== false
        ) {
          setSkillsRevision((current) => current + 1);
        }
        const refs = collectArtifactReferences(
          event.payload.result.metadata,
          event.payload.result.output,
        );
        if (refs.length > 0) {
          setArtifacts((current) =>
            mergeArtifactDescriptors(current, refs, event),
          );
        }
      }

      if (event.payload.type === "context_compacted") {
        const latestSummary = event.payload.summary;
        setContextStatus((current) => ({
          budget: current?.budget ?? {
            totalTokens: 128000,
            usedTokens: 0,
            messageCount: 0,
            estimatedUsage: 0,
          },
          latestSummary,
          usage: current?.usage ?? emptyContextUsage(),
          projection: current?.projection,
        }));
      }

      if (event.payload.type === "context_projection_built") {
        const projection = event.payload.projection;
        setContextStatus((current) =>
          current ? { ...current, projection } : current,
        );
      }

      if (event.payload.type === "provider_context_state_updated") {
        const providerContextState = event.payload;
        setContextStatus((current) => {
          if (!current) return current;
          return {
            ...current,
            projection: current.projection
              ? {
                  ...current.projection,
                  providerStateAvailable: true,
                  providerStateKind: providerContextState.state_kind,
                  providerItemCount: providerContextState.response_item_count,
                  nativeCompactionItemCount:
                    providerContextState.compaction_item_count,
                }
              : undefined,
          };
        });
      }

      if (event.payload.type === "provider_context_state_invalidated") {
        setContextStatus((current) => {
          if (!current) return current;
          return {
            ...current,
            usage: {
              ...current.usage,
              providerFallbacks: current.usage.providerFallbacks + 1,
            },
            projection: current.projection
              ? {
                  ...current.projection,
                  providerStateAvailable: false,
                  providerStateKind: null,
                  providerItemCount: 0,
                  nativeCompactionItemCount: 0,
                }
              : current.projection,
          };
        });
      }
    },
    [
      conversationRegistry,
      deliverTaskCompletionNotification,
      markThreadActivityRead,
    ],
  );
  conversationEventEffectRef.current = ingestEvent;

  const commitConversationEventTraces = useCallback((events: AgentEvent[]) => {
    const pendingTraces = pendingEventCommitTraceRef.current;
    let oldestPendingSeq = Number.POSITIVE_INFINITY;
    for (const pendingTrace of pendingTraces.values()) {
      oldestPendingSeq = Math.min(oldestPendingSeq, pendingTrace.eventSeq);
    }
    let latestStreamSeq = -1;
    for (let index = events.length - 1; index >= 0; index -= 1) {
      const event = events[index];
      if (!event || event.seq === Number.MAX_SAFE_INTEGER) continue;
      if (latestStreamSeq < 0) latestStreamSeq = event.seq;
      if (event.seq < oldestPendingSeq) break;
      const pendingTrace = pendingTraces.get(event.id);
      if (!pendingTrace) continue;
      pendingTraces.delete(event.id);
      const traceTime = rendererTraceTime();
      recordConversationRenderTrace({
        stage: "committed",
        ...pendingTrace.eventTrace,
        ...traceTime,
        latencyMs: Math.max(
          0,
          traceTime.rendererClockMs - pendingTrace.receivedClockMs,
        ),
        change: "append",
        textLength: pendingTrace.eventTrace.text.length,
      });
    }
    for (const [eventId, pendingTrace] of pendingTraces) {
      if (pendingTrace.eventSeq <= latestStreamSeq) {
        pendingTraces.delete(eventId);
      }
    }
  }, []);

  const ingestTerminalEvent = useCallback((event: TerminalEvent) => {
    setTerminalEvents((current) => {
      if (current.some((item) => item.id === event.id)) return current;
      return [...current, event].sort((a, b) => a.seq - b.seq);
    });
    if (
      event.type === "finished" ||
      event.type === "cancelled" ||
      event.type === "error"
    ) {
      setTerminalSession((current) =>
        current?.sessionId === event.commandId ? null : current,
      );
    }
  }, []);

  useEffect(() => {
    if (activeThread?.workspaceRoot) {
      setSelectedWorkspaceRoot(activeThread.workspaceRoot);
    }
  }, [activeThread?.workspaceRoot]);

  useEffect(() => {
    let cancelled = false;
    setServerProbing(true);
    const bootstrapping = loadPlatformInfo().then(async (info) => {
      if (cancelled) return;
      const nextClient = new ApiClient(info.backendUrl);
      setPlatform(info);
      // Retries reuse the existing client so effects keyed on it do not re-fire
      // once every probe; only a reassigned backend port swaps the instance.
      const sameEndpoint = clientEndpointRef.current === info.backendUrl;
      clientEndpointRef.current = info.backendUrl;
      if (!sameEndpoint) threadActivityStore.reset();
      setClient((current) => (current && sameEndpoint ? current : nextClient));

      try {
        const sources = await listSecretSources();
        if (!cancelled) setSecretSources(sources);
      } catch (error) {
        if (!cancelled) {
          setWorkspaceError(
            error instanceof Error ? error.message : String(error),
          );
        }
      }

      try {
        await nextClient.health();
        let [
          loadedProjects,
          loadedThreads,
          loadedSettings,
          loadedHealth,
          loadedMcp,
        ] = await Promise.all([
          nextClient.listProjects(),
          nextClient.listThreads(),
          nextClient.getSettings(),
          nextClient.getProviderHealth(),
          nextClient.listMcpServers(),
        ]);

        if (
          loadedSettings.permissionMode === "chat" ||
          loadedSettings.permissionMode === "read_only"
        ) {
          loadedSettings = await nextClient.updateSettings({
            permissionMode: "auto",
            sandbox: controlledSandboxSettings(loadedSettings.sandbox),
          });
        }

        try {
          const migrated = await migrateLegacyProjectData(
            nextClient,
            loadedProjects,
            loadedThreads,
          );
          loadedProjects = migrated.projects;
          loadedThreads = migrated.threads;
        } catch (error) {
          if (!cancelled) {
            setActionError(`旧项目数据迁移失败：${errorMessage(error)}`);
          }
        }

        loadedThreads = await nextClient.listThreads(true, experienceMode);

        if (cancelled) return;
        setProjects(sortProjects(loadedProjects));
        setThreads(loadedThreads);
        const activityBaseline =
          threadActivityStore.captureLiveReconciliationBaseline();
        void nextClient
          .listActivityStatuses()
          .then((turnStatuses) => {
            if (cancelled) return;
            threadActivityStore.retainKnownThreads(
              new Set(loadedThreads.map((thread) => thread.id)),
            );
            threadActivityStore.reconcileLiveTurnStatuses(
              turnStatuses,
              activityBaseline,
            );
          })
          .catch(() => undefined);
        setSettings(loadedSettings);
        setProviderHealth(loadedHealth);
        setMcpServers(loadedMcp);
        const projectIds = new Set(loadedProjects.map((project) => project.id));
        const lastActiveThreadId = readLastActiveThreadId(experienceMode);
        const restoredThread = loadedThreads.find(
          (thread) =>
            thread.id === lastActiveThreadId &&
            !thread.archivedAt &&
            thread.experienceMode === experienceMode,
        );
        const firstVisibleThread = loadedThreads.find(
          (thread) =>
            !thread.archivedAt &&
            thread.experienceMode === experienceMode &&
            thread.projectId &&
            projectIds.has(thread.projectId),
        );
        const firstProject = sortProjects(loadedProjects)[0] ?? null;
        const initialThread = restoredThread ?? firstVisibleThread ?? null;
        setActiveThreadId((current) => current ?? initialThread?.id ?? null);
        if (!initialThread) {
          setDraftProjectId((current) => current ?? firstProject?.id ?? null);
        }
        setSelectedWorkspaceRoot(
          (current) =>
            current ??
            initialThread?.workspaceRoot ??
            firstProject?.workspaceRoot ??
            null,
        );
        setServerStatus("online");
        setServerError(null);
        setServerProbing(false);
      } catch (error) {
        if (cancelled) return;
        setServerStatus("offline");
        setServerError(error instanceof Error ? error.message : String(error));
        setServerProbing(false);
      }
    });
    void bootstrapping.catch((error) => {
      if (cancelled) return;
      setServerStatus("offline");
      setServerError(error instanceof Error ? error.message : String(error));
      setServerProbing(false);
    });

    return () => {
      cancelled = true;
    };
  }, [bootstrapRetryNonce, threadActivityStore]);

  useEffect(() => {
    const hasCodexProvider = Boolean(
      client &&
      settings?.providers.some(
        (provider) => provider.kind === "codex_app_server",
      ),
    );
    if (!hasCodexProvider) {
      setCodexAccount(null);
      setCodexAccountError(null);
      return;
    }
    void refreshCodexAccount();
  }, [client, refreshCodexAccount, settings?.providers]);

  useEffect(() => {
    if (!client || !codexAccount?.loginPending) return;
    const timer = window.setInterval(() => {
      void refreshCodexAccount();
    }, 2000);
    return () => window.clearInterval(timer);
  }, [client, codexAccount?.loginPending, refreshCodexAccount]);

  // The main process owns backend startup and reports its phase to the renderer.
  // Do not poll/retry while Cargo is still compiling: those probes cannot make
  // the build finish and only create misleading retry activity. Once the
  // managed backend reports ready, perform one normal workspace bootstrap.
  useEffect(() => {
    const readyAt =
      backendStartupStatus?.phase === "ready"
        ? backendStartupStatus.updatedAt
        : null;
    if (
      !readyAt ||
      serverStatus !== "offline" ||
      readyStartupBootstrapRef.current === readyAt
    ) {
      return;
    }
    readyStartupBootstrapRef.current = readyAt;
    setBootstrapRetryNonce((nonce) => nonce + 1);
  }, [backendStartupStatus, serverStatus]);

  // New tasks start on the newest stable model of the active connection. This is
  // why there is no "quality vs. speed" preference to configure: the latest
  // model is the default, and the composer is where you deviate from it.
  useEffect(() => {
    if (!settings) return;
    setDraftModelSelection((current) =>
      resolveDraftModelSelection(
        settings.providers,
        settings.activeProviderId,
        current,
      ),
    );
  }, [settings]);

  useEffect(() => {
    writeDraftModelSelection(draftModelSelection);
  }, [draftModelSelection]);

  useEffect(() => {
    setGoalSnapshot(null);
    setAgentItems([]);
    agentRefreshRequestRef.current = null;
    if (!client || !activeThreadId) return;

    const sessionClient = client;
    const threadId = activeThreadId;
    const controller = new AbortController();
    let cancelled = false;
    let agentSource: StreamHandle | null = null;
    let agentRefreshTimer: ReturnType<typeof setTimeout> | null = null;
    let agentRefreshInFlight = false;
    let agentRefreshQueued = false;

    function openAgentStream(items: AgentListItem[]) {
      if (agentSource || items.length === 0) return;
      const cursor = items.reduce(
        (latest, item) => Math.max(latest, item.activity?.cursor ?? 0),
        0,
      );
      agentSource = sessionClient.openAgentEventStream(
        threadId,
        cursor || undefined,
        scheduleAgentRefresh,
      );
    }

    function refreshAgents() {
      if (agentRefreshInFlight) {
        agentRefreshQueued = true;
        return;
      }
      agentRefreshInFlight = true;
      void sessionClient
        .listAgents(threadId, controller.signal)
        .then((items) => {
          if (cancelled) return;
          setAgentItems((current) => reuseUnchangedAgentList(current, items));
          openAgentStream(items);
        })
        .catch((error) => {
          if (!cancelled && !isAbortError(error)) {
            setServerError(errorMessage(error));
          }
        })
        .finally(() => {
          agentRefreshInFlight = false;
          if (agentRefreshQueued && !cancelled) {
            agentRefreshQueued = false;
            scheduleAgentRefresh();
          }
        });
    }

    function scheduleAgentRefresh() {
      if (agentRefreshTimer) return;
      agentRefreshTimer = setTimeout(() => {
        agentRefreshTimer = null;
        refreshAgents();
      }, 150);
    }

    agentRefreshRequestRef.current = scheduleAgentRefresh;
    refreshAgents();
    void sessionClient
      .getGoal(threadId, controller.signal)
      .then((snapshot) => {
        if (!cancelled) setGoalSnapshot(snapshot);
      })
      .catch((error) => {
        if (!cancelled && !isAbortError(error))
          setServerError(errorMessage(error));
      });

    return () => {
      cancelled = true;
      controller.abort();
      agentSource?.close();
      if (agentRefreshTimer) clearTimeout(agentRefreshTimer);
      if (agentRefreshRequestRef.current === scheduleAgentRefresh) {
        agentRefreshRequestRef.current = null;
      }
    };
  }, [activeThreadId, client]);

  useEffect(() => {
    if (!client || !activeThreadId || !terminalToolActive) {
      setTerminalEvents([]);
      setTerminalSession(null);
      return;
    }
    let cancelled = false;
    let source: StreamHandle | null = null;
    const controller = new AbortController();
    setTerminalEvents([]);
    setTerminalSession(null);

    void (async () => {
      const history = await client.listTerminalHistory(
        activeThreadId,
        undefined,
        controller.signal,
      );
      if (cancelled) return;
      const session = await client.ensureTerminalSession(activeThreadId);
      if (cancelled) return;

      // Windows console shells may emit a cursor-position query (ESC[6n)
      // before drawing the first prompt. Commit the writable session before
      // replaying history so xterm's automatic reply is not dropped.
      setTerminalSession(session);
      setTerminalEvents(history);
      const since = history.at(-1)?.seq;
      source = client.openTerminalStream(
        activeThreadId,
        since,
        ingestTerminalEvent,
      );
    })().catch((error) => {
      if (!cancelled && !isAbortError(error))
        setWorkbenchError(
          error instanceof Error ? error.message : String(error),
        );
    });

    return () => {
      cancelled = true;
      controller.abort();
      source?.close();
    };
  }, [activeThreadId, client, ingestTerminalEvent, terminalToolActive]);

  const refreshWorkbench = useCallback(
    async (path?: string) => {
      if (!client || !activeThreadId) return;
      workbenchRefreshControllerRef.current?.abort();
      const controller = new AbortController();
      workbenchRefreshControllerRef.current = controller;
      const threadId = activeThreadId;
      setIsRefreshingWorkbench(true);
      setWorkbenchError(null);
      try {
        const [
          tree,
          diff,
          sandboxStatus,
          threadMcp,
          artifactList,
          loadedContextStatus,
          loadedMcpServers,
        ] = await Promise.all([
          client.listWorkspaceTree(threadId, path, controller.signal),
          client.getWorkspaceDiff(threadId, controller.signal),
          client.getSandbox(threadId, controller.signal),
          client.listThreadMcpServers(threadId, controller.signal),
          client.listArtifacts(threadId, controller.signal),
          client.getContextStatus(threadId, controller.signal),
          client.listMcpServers(controller.signal),
        ]);
        if (controller.signal.aborted) return;
        setWorkspaceTree(tree);
        setWorkspaceDiff(diff);
        setSandbox(sandboxStatus);
        setThreadMcpServers(threadMcp);
        setArtifacts(artifactList);
        setContextStatus(loadedContextStatus);
        setMcpServers(loadedMcpServers);
      } catch (error) {
        if (isAbortError(error)) return;
        setWorkbenchError(
          error instanceof Error ? error.message : String(error),
        );
      } finally {
        if (workbenchRefreshControllerRef.current === controller) {
          workbenchRefreshControllerRef.current = null;
          setIsRefreshingWorkbench(false);
        }
      }
    },
    [activeThreadId, client],
  );

  useEffect(() => {
    if (!activeThreadId || !isConversationReady) {
      workbenchRefreshControllerRef.current?.abort();
      workbenchRefreshControllerRef.current = null;
      setWorkspaceTree(null);
      setWorkspaceDiff(null);
      setSandbox(null);
      setThreadMcpServers([]);
      setFilePreview(null);
      setArtifacts([]);
      setContextStatus(null);
      return;
    }
    void refreshWorkbench();
    return () => {
      workbenchRefreshControllerRef.current?.abort();
    };
  }, [activeThreadId, isConversationReady, refreshWorkbench]);

  function selectThread(threadId: string) {
    const thread = threads.find((item) => item.id === threadId);
    markThreadActivityRead(threadId);
    activeThreadIdRef.current = threadId;
    startTransition(() => {
      setActiveThreadId(threadId);
      navigateFlowPrimaryView("conversation");
      if (activeToolTab?.kind === "extensions") setToolStageOpen(false);
      if (thread) setExperienceMode(thread.experienceMode);
      setDraftProjectId(null);
      if (thread?.workspaceRoot) setSelectedWorkspaceRoot(thread.workspaceRoot);
    });
  }

  useEffect(() => {
    const nativeApi = window.opentopia;
    if (!nativeApi || !client) return;

    let cancelled = false;
    const openThread = async (request: { threadId?: string }) => {
      if (!request.threadId || cancelled) return;
      try {
        const nextThreads = await client.listThreads(true);
        if (cancelled) return;
        setThreads(nextThreads);
        const thread = nextThreads.find((item) => item.id === request.threadId);
        if (!thread) return;
        markThreadActivityRead(thread.id);
        activeThreadIdRef.current = thread.id;
        setActiveThreadId(thread.id);
        navigateFlowPrimaryView("conversation");
        setExperienceMode(thread.experienceMode);
        setDraftProjectId(null);
        setSelectedWorkspaceRoot(thread.workspaceRoot);
      } catch (error) {
        console.warn("OpenTopia could not open the requested task", error);
      }
    };

    const unsubscribe = nativeApi.onOpenRequest((request) => {
      void openThread(request);
    });
    void nativeApi
      .getOpenRequests()
      .then((requests) => {
        const latest = [...requests]
          .reverse()
          .find((request) => request.threadId);
        if (latest) void openThread(latest);
      })
      .catch(() => {});

    return () => {
      cancelled = true;
      unsubscribe();
    };
  }, [client]);

  function prepareNewThread(
    workspaceRoot: string | null,
    projectId: string | null = null,
  ) {
    activeThreadIdRef.current = null;
    setActiveThreadId(null);
    navigateFlowPrimaryView("conversation");
    setNewTaskLaunchMode("local");
    clearToolTabs();
    setActiveToolTabId(null);
    setToolStageOpen(false);
    setConversationCollapsed(false);
    setDraftFlowLibraryProvider(null);
    setSelectedWorkspaceRoot(workspaceRoot);
    setDraftProjectId(projectId);
  }

  function changeExperienceMode(nextMode: ExperienceMode) {
    if (nextMode === experienceMode) return;
    const project = activeProject ?? draftProject;
    setExperienceMode(nextMode);
    if (nextMode !== "flow") navigateFlowPrimaryView("conversation");
    if (client) {
      void client
        .listThreads(true, nextMode)
        .then(setThreads)
        .catch((error) => {
          setActionError(
            `加载 ${nextMode} 模式任务失败：${errorMessage(error)}`,
          );
        });
    }
    prepareNewThread(
      project?.workspaceRoot ?? currentWorkspaceRoot,
      project?.id ?? draftProjectId,
    );
  }

  function beginNewThread() {
    const project =
      activeProject ??
      projects.find(
        (item) =>
          item.workspaceRoot &&
          currentWorkspaceRoot &&
          workspaceRootKey(item.workspaceRoot) ===
            workspaceRootKey(currentWorkspaceRoot),
      ) ??
      null;
    prepareNewThread(project?.workspaceRoot ?? null, project?.id ?? null);
  }

  function beginProjectDraft(project: Project) {
    prepareNewThread(project.workspaceRoot, project.id);
  }

  function handleNewThreadForProject(project: Project) {
    beginProjectDraft(project);
  }

  async function createBlankProject(name: string): Promise<Project | null> {
    if (!client) return null;
    setActionError(null);
    try {
      const project = await client.createProject({ name });
      setProjects((current) => sortProjects([project, ...current]));
      return project;
    } catch (error) {
      setActionError(`创建项目失败：${errorMessage(error)}`);
      return null;
    }
  }

  async function toggleProjectPinned(project: Project) {
    if (!client) return;
    setActionError(null);
    try {
      const updated = await client.updateProject(project.id, {
        pinned: !project.pinned,
      });
      setProjects((current) =>
        sortProjects(
          current.map((item) => (item.id === updated.id ? updated : item)),
        ),
      );
    } catch (error) {
      setActionError(`更新项目失败：${errorMessage(error)}`);
    }
  }

  async function removeProject(project: Project) {
    if (!client) return;
    const confirmed = window.confirm(
      `归档项目“${project.name}”？项目会从列表移除，所属任务会归档，可在“已归档”中恢复。`,
    );
    if (!confirmed) return;

    setActionError(null);
    try {
      await client.deleteProject(project.id);
      const [nextProjects, nextThreads] = await Promise.all([
        client.listProjects(),
        client.listThreads(true),
      ]);
      const sortedProjects = sortProjects(nextProjects);
      setProjects(sortedProjects);
      setThreads(nextThreads);
      if (
        draftProjectId === project.id ||
        activeThread?.projectId === project.id
      ) {
        const nextProject = sortedProjects[0] ?? null;
        prepareNewThread(nextProject?.workspaceRoot ?? null, nextProject?.id);
      }
    } catch (error) {
      setActionError(`归档项目失败：${errorMessage(error)}`);
    }
  }

  async function restoreThread(thread: Thread) {
    if (!client) return;
    setActionError(null);
    try {
      let targetProject = thread.projectId
        ? (projects.find((project) => project.id === thread.projectId) ?? null)
        : null;
      targetProject ??=
        projects.find(
          (project) =>
            project.workspaceRoot &&
            workspaceRootKey(project.workspaceRoot) ===
              workspaceRootKey(thread.workspaceRoot),
        ) ?? null;
      if (!targetProject) {
        targetProject = await client.createProject({
          name: workspaceName(thread.workspaceRoot),
          workspaceRoot: thread.workspaceRoot,
        });
        setProjects((current) => sortProjects([targetProject!, ...current]));
      }

      const restored = await client.updateThread(thread.id, {
        projectId: targetProject.id,
        archivedAt: null,
      });
      setThreads((current) =>
        current.map((item) => (item.id === restored.id ? restored : item)),
      );
      selectThread(restored.id);
    } catch (error) {
      setActionError(`恢复任务失败：${errorMessage(error)}`);
    }
  }

  function openToolTab(kind: ToolTabKind) {
    if (kind === "image" || kind === "preview" || kind === "side-task") return;
    if (kind === "browser") {
      openSharedBrowserTab();
      return;
    }
    const id = `tool-${kind}`;
    setToolTabs((current) =>
      current.some((tab) => tab.id === id)
        ? current
        : [...current, { id, kind, title: toolTabTitle(kind) }],
    );
    setActiveToolTabId(id);
    setToolStageOpen(true);
    setConversationCollapsed(false);
  }

  function openNewBrowserTab(initialUrl?: string) {
    const browserHost = window.opentopia?.browserHost;
    if (!browserHost) {
      openSharedBrowserTab();
      return;
    }
    const sessionId = newBrowserTabSessionId();
    const launchGeneration = browserTabLaunchGenerationRef.current;
    void initializeBrowserTabSession(browserHost, sessionId, initialUrl)
      .then((openedUrl) => {
        if (launchGeneration !== browserTabLaunchGenerationRef.current) {
          void browserHost.destroySession(sessionId).catch(() => {});
          return;
        }
        const id = `tool-browser:${sessionId}`;
        const sequence = ++browserTabSequenceRef.current;
        const fallbackTitle = `浏览器 ${sequence}`;
        setToolTabs((current) => [
          ...current,
          {
            id,
            kind: "browser",
            title: openedUrl
              ? browserTabTitle({ url: openedUrl }, fallbackTitle)
              : fallbackTitle,
            browserSessionId: sessionId,
          },
        ]);
        setActiveToolTabId(id);
        setToolStageOpen(true);
        setConversationCollapsed(false);
      })
      .catch((error: unknown) => {
        void browserHost.destroySession(sessionId).catch(() => {});
        if (launchGeneration === browserTabLaunchGenerationRef.current) {
          setActionError(`无法新建浏览器：${errorMessage(error)}`);
        }
      });
  }

  browserNewTabRequestHandlerRef.current = ({ openerSessionId, url }) => {
    const openerStillExists = toolTabsRef.current.some(
      (tab) =>
        tab.browserSessionId === openerSessionId ||
        (tab.id === "tool-browser" &&
          browserSessionId(activeThreadIdRef.current) === openerSessionId),
    );
    if (openerStillExists) openNewBrowserTab(url);
  };

  function openSharedBrowserTab() {
    setToolTabs((current) =>
      current.some((tab) => tab.id === "tool-browser")
        ? current
        : [
            ...current,
            {
              id: "tool-browser",
              kind: "browser",
              title: toolTabTitle("browser"),
            },
          ],
    );
    setActiveToolTabId("tool-browser");
    setToolStageOpen(true);
    setConversationCollapsed(false);
  }

  const updateBrowserTabState = useCallback(
    (tabId: string, state: WebPreviewState) => {
      setToolTabs((current) =>
        current.map((tab) => {
          if (tab.id !== tabId || tab.kind !== "browser") return tab;
          const title = browserTabTitle(state, tab.title);
          const browserFaviconUrl = state.faviconUrl ?? undefined;
          return title === tab.title &&
            tab.browserFaviconUrl === browserFaviconUrl
            ? tab
            : { ...tab, title, browserFaviconUrl };
        }),
      );
    },
    [],
  );

  async function openSideTask() {
    if (!client || !activeThread) return;

    const tabId = `side-task:${crypto.randomUUID()}`;
    setToolTabs((current) => [
      ...current,
      { id: tabId, kind: "side-task", title: "侧边任务" },
    ]);
    setActiveToolTabId(tabId);
    setToolStageOpen(true);
    setConversationCollapsed(false);
    setActionError(null);

    try {
      let sideThread = await client.createThread({
        title: "侧边任务",
        workspaceRoot: activeThread.workspaceRoot,
        projectId: activeThread.projectId ?? undefined,
        experienceMode: activeThread.experienceMode,
      });
      if (activeThread.modelSelection) {
        try {
          sideThread = await client.setThreadModel(
            sideThread.id,
            activeThread.modelSelection,
          );
        } catch (error) {
          console.warn("OpenTopia could not pin the side task model", error);
        }
      }
      setThreads((current) => [
        sideThread,
        ...current.filter((thread) => thread.id !== sideThread.id),
      ]);
      setToolTabs((current) =>
        current.map((tab) =>
          tab.id === tabId ? { ...tab, sideTaskThreadId: sideThread.id } : tab,
        ),
      );
    } catch (error) {
      setToolTabs((current) => current.filter((tab) => tab.id !== tabId));
      setActiveToolTabId((current) => (current === tabId ? null : current));
      setActionError(`创建侧边任务失败：${errorMessage(error)}`);
    }
  }

  // Text files go to the diff review; binary files go to the format-aware
  // preview host, which can render spreadsheets, PDFs, and images.
  function openFileReview(path: string, file?: TurnFileChange) {
    if (file?.binary) {
      if (!activeThread) {
        setActionError("Open a task before opening a file.");
        return;
      }
      openPreviewTab(
        activeThread.id,
        { type: "workspace", path },
        markdownLinkTitle(path),
      );
      return;
    }
    setReviewFileRequest((current) => ({
      path,
      nonce: (current?.nonce ?? 0) + 1,
    }));
    openToolTab("diff");
    void refreshWorkbench();
  }

  function toggleToolPanel(kind: Exclude<ToolTabKind, "image" | "preview">) {
    if (kind === "browser") {
      openNewBrowserTab();
      return;
    }
    const tabId = `tool-${kind}`;
    if (toolStageOpen && activeToolTabId === tabId) {
      setToolStageOpen(false);
      setConversationCollapsed(false);
      return;
    }
    openToolTab(kind);
  }

  function releaseBrowserTabSession(tab: ToolTab | undefined) {
    const sessionId = tab?.browserSessionId;
    const browserHost = window.opentopia?.browserHost;
    if (!sessionId || !browserHost) return;
    void browserHost.destroySession(sessionId).catch(() => {
      // A tab can be closed before its native view finishes initializing, or
      // after Electron has already removed sessions while the window closes.
    });
  }

  function clearToolTabs() {
    browserTabLaunchGenerationRef.current += 1;
    for (const tab of toolTabsRef.current) releaseBrowserTabSession(tab);
    browserTabSequenceRef.current = 0;
    setToolTabs([]);
  }

  const openPreviewTab = useCallback(function openPreviewTab(
    threadId: string,
    target: PreviewTarget,
    title: string,
  ) {
    const targetKey =
      target.type === "workspace"
        ? `workspace:${target.path}`
        : target.type === "local"
          ? `local:${target.path}`
          : target.type === "artifact"
            ? `artifact:${target.artifactId}`
            : target.type === "attachment"
              ? `attachment:${target.attachmentId}`
              : `url:${target.url}`;
    const id = `preview:${threadId}:${targetKey}`;
    setToolTabs((current) =>
      current.some((tab) => tab.id === id)
        ? current
        : [...current, { id, kind: "preview", title, previewTarget: target }],
    );
    setActiveToolTabId(id);
    setToolStageOpen(true);
    setConversationCollapsed(false);
  }, []);

  const openInlineImagePreview = useCallback(function openInlineImagePreview(
    threadId: string,
    sourceId: string,
    image: ImagePreviewSource,
  ) {
    const id = `image-preview:${threadId}:${sourceId}`;
    const title = image.name?.trim() || "图片";
    setToolTabs((current) =>
      current.some((tab) => tab.id === id)
        ? current
        : [...current, { id, kind: "image", title, imagePreview: image }],
    );
    setActiveToolTabId(id);
    setToolStageOpen(true);
    setConversationCollapsed(false);
  }, []);

  const openMarkdownLink = useCallback(
    function openMarkdownLink(href: string, baseWorkspacePath?: string | null) {
      const target = resolveMarkdownLink(href, baseWorkspacePath);
      if (target.kind === "anchor") return;
      if (target.kind === "blocked") {
        setActionError(target.reason);
        return;
      }
      if (target.kind === "email") {
        void openExternal(target.url).catch((error) =>
          setActionError(
            error instanceof Error ? error.message : String(error),
          ),
        );
        return;
      }
      if (!activeThread) {
        setActionError("Open a task before following this link.");
        return;
      }
      if (target.kind === "workspace") {
        openPreviewTab(
          activeThread.id,
          { type: "workspace", path: target.path },
          markdownLinkTitle(target.path),
        );
        return;
      }
      if (target.kind === "local") {
        openPreviewTab(
          activeThread.id,
          { type: "local", path: target.path },
          markdownLinkTitle(target.path),
        );
        return;
      }

      const navigation: BrowserNavigationRequest = {
        id: `${activeThread.id}:${++markdownNavigationIdRef.current}`,
        url: target.url,
      };
      const id = "tool-browser";
      setToolTabs((current) => {
        const existing = current.find((tab) => tab.id === id);
        if (!existing) {
          return [
            ...current,
            {
              id,
              kind: "browser",
              title: toolTabTitle("browser"),
              browserNavigation: navigation,
            },
          ];
        }
        return current.map((tab) =>
          tab.id === id ? { ...tab, browserNavigation: navigation } : tab,
        );
      });
      setActiveToolTabId(id);
      setToolStageOpen(true);
      setConversationCollapsed(false);
    },
    [activeThread, openPreviewTab],
  );

  function closeToolTab(tabId: string) {
    if (
      previewSessionStore.isDirty(tabId) &&
      !window.confirm("关闭标签会丢弃尚未保存的 Markdown 更改，是否继续？")
    ) {
      return;
    }
    releaseBrowserTabSession(
      toolTabsRef.current.find((tab) => tab.id === tabId),
    );
    previewSessionStore.delete(tabId);
    setToolTabs((current) => {
      const next = closeToolTabState(current, activeToolTabId, tabId);
      if (next.activeTabId !== activeToolTabId) {
        setActiveToolTabId(next.activeTabId);
      }
      if (next.shouldCollapse) {
        setToolStageOpen(false);
        setConversationCollapsed(false);
      }
      return next.tabs;
    });
  }

  async function chooseWorkspace(
    bindDraftProject = false,
  ): Promise<Project | null> {
    if (!client) return null;
    const projectToBind = bindDraftProject ? draftProject : null;
    setIsPickingWorkspace(true);
    setWorkspaceError(null);
    setActionError(null);
    try {
      const result = await selectWorkspace({
        defaultPath: currentWorkspaceRoot ?? undefined,
      });
      if (result.canceled) return null;

      const existingProject = projects.find(
        (project) =>
          project.workspaceRoot &&
          workspaceRootKey(project.workspaceRoot) ===
            workspaceRootKey(result.workspaceRoot),
      );
      if (existingProject) {
        if (projectToBind && existingProject.id !== projectToBind.id) {
          setActionError(
            `该工作区已绑定到项目“${existingProject.name}”，请先选择其他文件夹。`,
          );
          return null;
        }
        beginProjectDraft(existingProject);
        return existingProject;
      }

      const project = projectToBind
        ? await client.updateProject(projectToBind.id, {
            workspaceRoot: result.workspaceRoot,
          })
        : await client.createProject({
            name: result.workspace.name,
            workspaceRoot: result.workspaceRoot,
          });
      setProjects((current) =>
        sortProjects([
          project,
          ...current.filter((item) => item.id !== project.id),
        ]),
      );
      if (project.workspaceRoot) {
        setThreads((current) =>
          current.map((thread) =>
            thread.projectId === project.id
              ? { ...thread, workspaceRoot: project.workspaceRoot! }
              : thread,
          ),
        );
      }
      beginProjectDraft(project);
      return project;
    } catch (error) {
      setActionError(`选择工作区失败：${errorMessage(error)}`);
      return null;
    } finally {
      setIsPickingWorkspace(false);
    }
  }

  function selectProject(projectId: string) {
    const project = projects.find((item) => item.id === projectId);
    if (project) beginProjectDraft(project);
  }

  async function openWorkspaceRoot(workspaceRoot: string) {
    setWorkspaceError(null);
    try {
      await openPath(workspaceRoot);
    } catch (error) {
      setWorkspaceError(error instanceof Error ? error.message : String(error));
    }
  }

  async function openWorkspaceEntry(entry: WorkspaceEntry) {
    if (!client || !activeThread) return;
    setWorkbenchError(null);
    try {
      if (entry.kind === "directory") {
        setFilePreview(null);
        await refreshWorkbench(entry.path);
      } else if (entry.kind === "file") {
        setFilePreview(null);
        if (usesFormatAwarePreview(entry.path)) {
          openPreviewTab(
            activeThread.id,
            { type: "workspace", path: entry.path },
            entry.name,
          );
          return;
        }
        try {
          setFilePreview(
            await client.readWorkspaceFile(activeThread.id, entry.path),
          );
        } catch {
          openPreviewTab(
            activeThread.id,
            { type: "workspace", path: entry.path },
            entry.name,
          );
        }
      }
    } catch (error) {
      setWorkbenchError(error instanceof Error ? error.message : String(error));
    }
  }

  async function openWorkspacePath(path?: string) {
    setFilePreview(null);
    await refreshWorkbench(path);
  }

  async function toggleThreadMcp(serverId: string, enabled: boolean) {
    if (!client || !activeThread) return;
    setWorkbenchError(null);
    try {
      await client.setThreadMcpServer(activeThread.id, serverId, enabled);
      setThreadMcpServers(await client.listThreadMcpServers(activeThread.id));
    } catch (error) {
      setWorkbenchError(error instanceof Error ? error.message : String(error));
    }
  }

  async function refreshMcpState() {
    if (!client) return;
    const [servers, bindings] = await Promise.all([
      client.listMcpServers(),
      activeThread
        ? client.listThreadMcpServers(activeThread.id)
        : Promise.resolve([]),
    ]);
    setMcpServers(servers);
    setThreadMcpServers(bindings);
  }

  async function createMcpServer(input: McpServerInput) {
    if (!client) throw new Error("OpenTopia API is unavailable.");
    await client.createMcpServer(input);
    await refreshMcpState();
  }

  async function updateMcpServer(serverId: string, input: McpServerInput) {
    if (!client) throw new Error("OpenTopia API is unavailable.");
    await client.updateMcpServer(serverId, input);
    await refreshMcpState();
  }

  async function restartMcpServer(serverId: string) {
    if (!client) throw new Error("OpenTopia API is unavailable.");
    await client.restartMcpServer(serverId);
    await refreshMcpState();
  }

  async function deleteMcpServer(serverId: string) {
    if (!client) throw new Error("OpenTopia API is unavailable.");
    await client.deleteMcpServer(serverId);
    await refreshMcpState();
  }

  async function refreshPluginState() {
    if (!client) return;
    const [availablePlugins, availableSkills, servers, bindings] =
      await Promise.all([
        client.listPlugins({
          workspaceRoot: currentWorkspaceRoot,
          threadId: activeThread?.id,
        }),
        client.listSkills(
          currentWorkspaceRoot,
          activeThread?.id,
          experienceMode,
        ),
        client.listMcpServers(),
        activeThread
          ? client.listThreadMcpServers(activeThread.id)
          : Promise.resolve([]),
      ]);
    setPlugins(availablePlugins);
    setSkills(availableSkills);
    setMcpServers(servers);
    setThreadMcpServers(bindings);
    const availableIds = new Set(availableSkills.map((skill) => skill.id));
    setSelectedSkillIds((current) =>
      current.filter((id) => availableIds.has(id)),
    );
  }

  async function installLocalPlugin() {
    if (!client) throw new Error("OpenTopia API is unavailable.");
    const selection = await selectPluginDirectory({
      defaultPath: currentWorkspaceRoot ?? undefined,
    });
    if (selection.canceled) return;
    await client.installPlugin(selection.path);
    await refreshPluginState();
  }

  async function uninstallLocalPlugin(pluginId: string) {
    if (!client) throw new Error("OpenTopia API is unavailable.");
    await client.uninstallPlugin(pluginId, currentWorkspaceRoot);
    await refreshPluginState();
  }

  async function toggleThreadPlugin(pluginId: string, enabled: boolean) {
    if (!client || !activeThread) {
      throw new Error("Open a task before enabling plugin tools.");
    }
    await client.setThreadPlugin(activeThread.id, pluginId, enabled);
    await refreshPluginState();
  }

  function usePluginSkills(pluginId: string, enabled: boolean) {
    const plugin = plugins.find((item) => item.plugin.id === pluginId);
    if (!plugin) return;
    if (!enabled) {
      setSelectedSkillIds((current) =>
        current.filter((id) => !plugin.skillIds.includes(id)),
      );
      return;
    }
    const next = [...selectedSkillIds];
    for (const skillId of plugin.skillIds) {
      if (next.length >= 5) break;
      if (!next.includes(skillId)) next.push(skillId);
    }
    if (plugin.skillIds.some((id) => !next.includes(id))) {
      setActionError("每轮最多选择 5 个 Skills；已添加当前可用的插件 Skills。");
    }
    setSelectedSkillIds(next);
  }

  async function saveSettings(input: {
    providers?: ProviderSettings[];
    activeProviderId?: string;
    providerKind?: ProviderKind;
    baseUrl?: string;
    model?: string;
    apiKeySource?: string;
    permissionMode?: PermissionMode;
    agentRuntime?: AppSettings["agentRuntime"];
    sandbox?: AppSettings["sandbox"];
  }) {
    if (!client) return false;
    setIsSavingSettings(true);
    try {
      const updated = await client.updateSettings(input);
      setSettings(updated);
      setProviderHealth(await client.getProviderHealth());
      if (activeThread) setSandbox(await client.getSandbox(activeThread.id));
      return true;
    } catch (error) {
      setActionError(`保存设置失败：${errorMessage(error)}`);
      return false;
    } finally {
      setIsSavingSettings(false);
    }
  }

  function changeExecutionPreset(
    permissionMode: "auto" | "approve" | "unrestricted",
  ) {
    if (!settings || isSavingSettings || activeTurnId) return;
    if (
      permissionMode === "unrestricted" &&
      !window.confirm(
        "完整系统访问会关闭系统沙箱并跳过所有工具审批。确定继续吗？",
      )
    ) {
      return;
    }
    void saveSettings({
      permissionMode,
      sandbox:
        permissionMode === "unrestricted"
          ? {
              ...settings.sandbox,
              sandboxMode: "danger-full-access",
              enforcement: "disabled",
              network: "allow",
            }
          : controlledSandboxSettings(settings.sandbox),
    });
  }

  function changeSandboxMode(mode: AppSettings["sandbox"]["sandboxMode"]) {
    if (!settings) return;
    const danger = mode === "danger-full-access";
    const permissionMode =
      !danger &&
      (settings.permissionMode === "full_access" ||
        settings.permissionMode === "unrestricted")
        ? "auto"
        : settings.permissionMode;
    void saveSettings({
      permissionMode,
      sandbox: {
        ...settings.sandbox,
        sandboxMode: mode,
        enforcement: danger
          ? "disabled"
          : settings.sandbox.enforcement === "disabled"
            ? "enforce"
            : settings.sandbox.enforcement,
        network: danger ? "allow" : settings.sandbox.network,
      },
    });
  }

  async function addContextSources(
    files?: File[],
  ): Promise<ContextSourceFile[]> {
    setActionError(null);
    try {
      const result = files
        ? await getDroppedContextFiles(files)
        : await selectContextFiles({
            defaultPath: currentWorkspaceRoot ?? undefined,
          });
      if (result.canceled) return [];
      setContextSources((current) => {
        const byPath = new Map(
          current.map((source) => [workspaceRootKey(source.path), source]),
        );
        for (const source of result.files) {
          byPath.set(workspaceRootKey(source.path), source);
        }
        return [...byPath.values()].slice(0, 20);
      });
      return result.files;
    } catch (error) {
      setActionError(`添加来源失败：${errorMessage(error)}`);
      return [];
    }
  }

  function removeContextSource(path: string) {
    const key = workspaceRootKey(path);
    setContextSources((current) =>
      current.filter((source) => workspaceRootKey(source.path) !== key),
    );
  }

  function toggleSkill(skillId: string) {
    setSelectedSkillIds((current) => {
      if (current.includes(skillId)) {
        return current.filter((id) => id !== skillId);
      }
      return current.length >= 5 ? current : [...current, skillId];
    });
  }

  async function interruptAgent(agentThreadId: string) {
    if (!client || !activeThread) return;
    setActionError(null);
    try {
      await client.interruptAgent(activeThread.id, agentThreadId);
    } catch (error) {
      setActionError(`中断 Agent 失败：${errorMessage(error)}`);
    }
  }

  async function runDirectToolCommand(
    threadId: string,
    command: DirectToolCommand,
  ) {
    if (!client) return;
    setWorkbenchError(null);
    if (command.kind === "run") {
      openToolTab("terminal");
      await client.startTerminalCommand(threadId, command.command);
      return;
    }

    openToolTab("files");
    setFilePreview(await client.readWorkspaceFile(threadId, command.path));
  }

  async function createThread(
    initialPrompt?: string,
    imageAttachments: InlineImageAttachment[] = [],
    contentParts: InlineMessageContentPart[] = [],
  ): Promise<boolean> {
    if (!client || isCreatingThread) return false;
    const submittedLibraryProvider =
      experienceMode === "flow" ? draftFlowLibraryProvider : null;
    if (newTaskLaunchMode === "new_worktree") {
      setActionError(
        "“新工作树”启动模式尚未接入线程创建；请选择“在本地处理”后继续。",
      );
      return false;
    }
    const directCommand = parseDirectToolCommand(initialPrompt ?? "");
    if (!directCommand && isLegacyDirectToolCommand(initialPrompt ?? "")) {
      setActionError("/run and /read require an argument.");
      return false;
    }
    if (
      directCommand &&
      (contextSources.length > 0 ||
        imageAttachments.length > 0 ||
        selectedSkillIds.length > 0)
    ) {
      setActionError("Direct tool commands cannot include agent context.");
      return false;
    }
    let project =
      activeProject ??
      projects.find(
        (item) =>
          item.workspaceRoot &&
          currentWorkspaceRoot &&
          workspaceRootKey(item.workspaceRoot) ===
            workspaceRootKey(currentWorkspaceRoot),
      ) ??
      null;
    if (!project?.workspaceRoot) project = await chooseWorkspace(true);
    if (!project?.workspaceRoot) return false;

    const shouldSendInitialPrompt =
      Boolean(initialPrompt?.trim()) ||
      contextSources.length > 0 ||
      imageAttachments.length > 0 ||
      selectedSkillIds.length > 0;
    const submittedContextPaths = contextSources.map((source) => source.path);
    const submittedSkillIds = [...selectedSkillIds];
    const submittedCollaborationMode = collaborationMode;
    const submittedModelSelection = draftModelSelection;
    let createdThreadId: string | null = null;
    setIsCreatingThread(true);
    setActionError(null);
    try {
      const prompt = initialPrompt?.trim() ?? "";
      let thread = await client.createThread({
        title: prompt ? threadTitleFromPrompt(prompt) : project.name,
        workspaceRoot: project.workspaceRoot,
        projectId: project.id,
        experienceMode,
      });
      createdThreadId = thread.id;
      setFlowLibraryBindings((current) =>
        updateFlowLibraryBindings(current, thread.id, submittedLibraryProvider),
      );
      setDraftFlowLibraryProvider(null);
      if (submittedModelSelection) {
        // Pin before the first turn runs, so the conversation starts on the
        // model picked in the draft composer rather than the connection default.
        try {
          thread = await client.setThreadModel(
            thread.id,
            submittedModelSelection,
          );
        } catch (error) {
          console.warn("OpenTopia could not pin the task model", error);
        }
      }
      setThreads((current) => [thread, ...current]);
      activeThreadIdRef.current = thread.id;
      setActiveThreadId(thread.id);
      setSelectedWorkspaceRoot(thread.workspaceRoot);
      setDraftProjectId(null);
      clearToolTabs();
      setActiveToolTabId(null);
      setToolStageOpen(false);
      if (directCommand) {
        await runDirectToolCommand(thread.id, directCommand);
        if (activeThreadIdRef.current === thread.id) setComposer("");
      } else if (shouldSendInitialPrompt) {
        markThreadActivityRead(thread.id);
        const sessionController = conversationRegistry?.get(thread.id);
        if (!sessionController) throw new Error("会话服务尚未就绪。");
        const result = await sessionController.send({
          content: initialPrompt?.trim() ?? "",
          sourcePaths: submittedContextPaths,
          skillIds: submittedSkillIds,
          collaborationMode: submittedCollaborationMode,
          imageAttachments,
          contentParts: imageAttachments.length > 0 ? contentParts : [],
          libraryProvider: submittedLibraryProvider ?? undefined,
        });
        if (!result) {
          throw new Error(
            sessionController.getSnapshot().commandError ?? "消息发送失败。",
          );
        }
        markThreadActivityRead(thread.id);
        if (activeThreadIdRef.current === thread.id) {
          setComposer("");
          setContextSources([]);
          setSelectedSkillIds([]);
        }
      }
      return activeThreadIdRef.current === thread.id;
    } catch (error) {
      if (
        createdThreadId === null ||
        activeThreadIdRef.current === createdThreadId
      ) {
        setActionError(`创建任务失败：${errorMessage(error)}`);
      }
      return false;
    } finally {
      setIsCreatingThread(false);
    }
  }

  async function submitRename(name: string): Promise<boolean> {
    if (!client || !renameTarget) return false;
    const trimmedName = name.trim();
    if (!trimmedName) return false;

    setActionError(null);
    try {
      if (renameTarget.kind === "project") {
        const updated = await client.updateProject(renameTarget.id, {
          name: trimmedName,
        });
        setProjects((current) =>
          sortProjects(
            current.map((project) =>
              project.id === updated.id ? updated : project,
            ),
          ),
        );
      } else {
        const updated = await client.updateThread(renameTarget.id, {
          title: trimmedName,
        });
        setThreads((current) =>
          current.map((thread) =>
            thread.id === updated.id ? updated : thread,
          ),
        );
      }
      setRenameTarget(null);
      return true;
    } catch (error) {
      setActionError(`重命名失败：${errorMessage(error)}`);
      return false;
    }
  }

  async function archiveThread(thread: Thread) {
    if (!client) return;
    setActionError(null);
    try {
      const archived = await client.updateThread(thread.id, {
        archivedAt: new Date().toISOString(),
      });
      const nextThreads = threads.map((item) =>
        item.id === archived.id ? archived : item,
      );
      setThreads(nextThreads);
      if (activeThreadId === thread.id) {
        const nextThread =
          nextThreads.find(
            (item) => !item.archivedAt && item.projectId === thread.projectId,
          ) ?? null;
        if (nextThread) {
          selectThread(nextThread.id);
        } else {
          const project =
            projects.find((item) => item.id === thread.projectId) ?? null;
          prepareNewThread(project?.workspaceRoot ?? null, project?.id ?? null);
        }
      }
    } catch (error) {
      setActionError(`归档任务失败：${errorMessage(error)}`);
    }
  }

  async function submitMessage(
    input: string,
    imageAttachments: InlineImageAttachment[] = [],
    contentParts: InlineMessageContentPart[] = [],
    collaborationModeOverride?: CollaborationMode,
  ): Promise<boolean> {
    const messageText = input.trim();
    if (
      !client ||
      !activeThread ||
      (!messageText &&
        contextSources.length === 0 &&
        selectedSkillIds.length === 0 &&
        imageAttachments.length === 0) ||
      isSending ||
      activeApproval ||
      activeUserInput
    )
      return false;
    const directCommand = parseDirectToolCommand(messageText);
    if (!directCommand && isLegacyDirectToolCommand(messageText)) {
      setActionError("/run and /read require an argument.");
      return false;
    }
    if (
      directCommand &&
      (contextSources.length > 0 ||
        imageAttachments.length > 0 ||
        selectedSkillIds.length > 0)
    ) {
      setActionError("Direct tool commands cannot include agent context.");
      return false;
    }
    const threadId = activeThread.id;
    const submittedContextPaths = contextSources.map((source) => source.path);
    const submittedSkillIds = [...selectedSkillIds];
    const submittedCollaborationMode =
      collaborationModeOverride ?? collaborationMode;
    const submittedGoalId = reusableGoalId(
      submittedCollaborationMode,
      goalSnapshot,
    );
    setActionError(null);
    activeConversationController?.clearCommandError();
    try {
      if (directCommand) {
        await runDirectToolCommand(threadId, directCommand);
        if (activeThreadIdRef.current === threadId) setComposer("");
        return activeThreadIdRef.current === threadId;
      }
      markThreadActivityRead(threadId);
      if (!activeConversationController) throw new Error("会话服务尚未就绪。");
      const result = await activeConversationController.send({
        content: messageText,
        sourcePaths: submittedContextPaths,
        skillIds: submittedSkillIds,
        collaborationMode: submittedCollaborationMode,
        goalId: submittedGoalId,
        imageAttachments,
        contentParts: imageAttachments.length > 0 ? contentParts : [],
        libraryProvider:
          activeThread.experienceMode === "flow"
            ? (flowLibraryBindings[threadId] ?? undefined)
            : undefined,
      });
      if (!result) {
        throw new Error(
          activeConversationController.getSnapshot().commandError ??
            "消息发送失败。",
        );
      }
      markThreadActivityRead(threadId);
      if (activeThreadIdRef.current === threadId) {
        setContextSources((current) =>
          current.length === submittedContextPaths.length &&
          current.every(
            (source, index) => source.path === submittedContextPaths[index],
          )
            ? []
            : current,
        );
        setSelectedSkillIds((current) =>
          current.length === submittedSkillIds.length &&
          current.every(
            (skillId, index) => skillId === submittedSkillIds[index],
          )
            ? []
            : current,
        );
      }
      return activeThreadIdRef.current === threadId;
    } catch (error) {
      if (activeThreadIdRef.current === threadId) {
        setActionError(errorMessage(error));
      }
      return false;
    }
  }

  async function cancelTurn() {
    if (!activeConversationController || !conversationTurnCanBeCancelled)
      return;
    setActionError(null);
    await activeConversationController.cancel();
    const error = activeConversationController.getSnapshot().commandError;
    if (error) setActionError(error);
  }

  async function runGoal() {
    if (!client || !activeThread || !goalSnapshot || activeTurnId) return;
    const goalId = goalSnapshot.goal.id;
    setGoalAction("run");
    setActionError(null);
    try {
      let snapshot = goalSnapshot;
      if (snapshot.workForm.status !== "active") {
        snapshot = await client.updateGoalStatus(
          activeThread.id,
          goalId,
          "active",
        );
        setGoalSnapshot(snapshot);
      }
      setCollaborationMode("goal");
      if (!activeConversationController) throw new Error("会话服务尚未就绪。");
      const result = await activeConversationController.send({
        content: "继续执行已确认的目标计划，直到完成或出现明确阻塞。",
        collaborationMode: "goal",
        goalId,
      });
      if (!result) {
        throw new Error(
          activeConversationController.getSnapshot().commandError ??
            "无法启动目标。",
        );
      }
    } catch (error) {
      setActionError(`无法启动目标：${errorMessage(error)}`);
    } finally {
      setGoalAction(null);
    }
  }

  async function changeGoalStatus(status: "paused" | "cancelled") {
    if (!client || !activeThread || !goalSnapshot) return;
    setGoalAction(status);
    setActionError(null);
    try {
      if (activeTurnId) {
        if (!activeConversationController)
          throw new Error("会话服务尚未就绪。");
        await activeConversationController.cancel();
        const cancelError =
          activeConversationController.getSnapshot().commandError;
        if (cancelError) throw new Error(cancelError);
      }
      const snapshot = await client.updateGoalStatus(
        activeThread.id,
        goalSnapshot.goal.id,
        status,
      );
      setGoalSnapshot(snapshot);
    } catch (error) {
      setActionError(`无法更新目标：${errorMessage(error)}`);
    } finally {
      setGoalAction(null);
    }
  }

  async function decideApproval(approvalId: string, approved: boolean) {
    if (!activeConversationController || decidingApprovalId) return;
    await activeConversationController.decideApproval(approvalId, approved);
  }

  async function submitUserInput(
    requestId: string,
    response: UserInputResponse,
  ) {
    if (!activeConversationController || submittingUserInputId) return;
    await activeConversationController.respondToUserInput(requestId, response);
  }

  async function ensureTerminalSession(
    threadId: string,
  ): Promise<TerminalSession> {
    if (!client) throw new Error("No client");
    const session = await client.ensureTerminalSession(threadId);
    setTerminalSession(session);
    return session;
  }

  async function writeTerminalSession(
    threadId: string,
    sessionId: string,
    data: string,
  ) {
    if (!client) return;
    try {
      await client.writeTerminalSession(threadId, sessionId, data);
    } catch (error) {
      setWorkbenchError(error instanceof Error ? error.message : String(error));
    }
  }

  async function resizeTerminalSession(
    threadId: string,
    sessionId: string,
    cols: number,
    rows: number,
  ) {
    if (!client) return;
    try {
      await client.resizeTerminalSession(threadId, sessionId, cols, rows);
    } catch (error) {
      setWorkbenchError(error instanceof Error ? error.message : String(error));
    }
  }

  async function closeTerminalSession(threadId: string, sessionId: string) {
    if (!client) return;
    try {
      await client.closeTerminalSession(threadId, sessionId);
      setTerminalSession((current) =>
        current?.sessionId === sessionId ? null : current,
      );
    } catch (error) {
      setWorkbenchError(error instanceof Error ? error.message : String(error));
    }
  }

  async function getArtifact(
    threadId: string,
    artifactId: string,
  ): Promise<ArtifactContent> {
    if (!client) throw new Error("No client");
    return client.getArtifact(threadId, artifactId);
  }

  function openArtifact(threadId: string, artifactId: string) {
    const descriptor = artifacts.find((artifact) => artifact.id === artifactId);
    openPreviewTab(
      threadId,
      { type: "artifact", artifactId },
      artifactPreviewTitle(descriptor, artifactId),
    );
  }

  async function compactContext() {
    if (!client || !activeThread || isCompactingContext) return;
    setIsCompactingContext(true);
    setWorkbenchError(null);
    try {
      await client.compactContext(activeThread.id);
      setContextStatus(await client.getContextStatus(activeThread.id));
    } catch (error) {
      setWorkbenchError(error instanceof Error ? error.message : String(error));
    } finally {
      setIsCompactingContext(false);
    }
  }

  async function revertDiffFile(path: string) {
    if (!client || !activeThread || revertingDiffPath) return;
    setRevertingDiffPath(path);
    setWorkbenchError(null);
    try {
      const result = await client.revertWorkspaceFile(
        activeThread.id,
        path,
        true,
      );
      setWorkspaceDiff(result.diff);
      setFilePreview(null);
      activeConversationController?.appendLocalEvent({
        id: `local-revert-${Date.now()}`,
        threadId: activeThread.id,
        turnId: null,
        seq: Number.MAX_SAFE_INTEGER,
        createdAt: new Date().toISOString(),
        payload: {
          type: "file_changed",
          path: result.path,
          summary: "File reverted from the Diff Review panel.",
        },
      });
    } catch (error) {
      setWorkbenchError(error instanceof Error ? error.message : String(error));
    } finally {
      setRevertingDiffPath(null);
    }
  }

  // Review panel plumbing: opening a changed file in its own tab, reading the
  // working-tree file behind a diff, and the per-turn diff baselines.
  function openReviewFileTab(path: string) {
    if (!activeThread) {
      setActionError("Open a task before opening a file.");
      return;
    }
    openPreviewTab(
      activeThread.id,
      { type: "workspace", path },
      markdownLinkTitle(path),
    );
  }

  async function loadReviewFileContent(
    path: string,
  ): Promise<DiffReviewFileContent> {
    if (!client || !activeThread) throw new Error("服务尚未连接。");
    const preview = await client.readWorkspaceFile(activeThread.id, path);
    return { content: preview.content, truncated: preview.truncated };
  }

  async function loadReviewTurnFileDiff(
    turnId: string,
    path: string,
  ): Promise<string> {
    if (!client || !activeThread) throw new Error("服务尚未连接。");
    const preview = await client.getTurnFileDiffPreview(
      activeThread.id,
      turnId,
      path,
    );
    return preview.diff;
  }

  async function runReviewGitAction(
    action: DiffReviewGitAction,
    message: string,
  ): Promise<string> {
    if (!client || !activeThread) throw new Error("服务尚未连接。");
    const threadId = activeThread.id;
    const notices: string[] = [];
    if (action === "commit" || action === "commit_push") {
      if (!message.trim()) throw new Error("请填写提交信息。");
      await client.runGitWorkflow(threadId, {
        type: "commit",
        request: { message, allTracked: true },
      });
      notices.push("提交已创建");
    }
    if (action === "push" || action === "commit_push") {
      const status = await client.getGitStatus(threadId);
      const branch = status.branch ?? "HEAD";
      await client.runGitWorkflow(threadId, {
        type: "push",
        request: { remote: "origin", branch, setUpstream: !status.upstream },
      });
      notices.push(`已推送 ${branch}`);
    }
    await refreshWorkbench();
    return notices.join("，");
  }

  async function openTurnUndo(turnId: string) {
    if (!client || !activeThread) return;
    const threadId = activeThread.id;
    setTurnUndoDialog({
      turnId,
      preview: null,
      loading: true,
      applying: false,
      error: null,
    });
    try {
      const preview = await client.previewTurnUndo(threadId, turnId);
      setTurnUndoDialog((current) =>
        current?.turnId === turnId
          ? { ...current, preview, loading: false }
          : current,
      );
    } catch (error) {
      setTurnUndoDialog((current) =>
        current?.turnId === turnId
          ? {
              ...current,
              loading: false,
              error: error instanceof Error ? error.message : String(error),
            }
          : current,
      );
    }
  }

  async function confirmTurnUndo() {
    if (
      !client ||
      !activeThread ||
      !turnUndoDialog?.preview?.canUndo ||
      turnUndoDialog.applying
    ) {
      return;
    }

    const threadId = activeThread.id;
    const turnId = turnUndoDialog.turnId;
    setTurnUndoDialog((current) =>
      current ? { ...current, applying: true, error: null } : current,
    );
    try {
      const result = await client.undoTurnChanges(threadId, turnId);
      if (!result.applied) {
        setTurnUndoDialog((current) =>
          current?.turnId === turnId
            ? { ...current, preview: result.preview, applying: false }
            : current,
        );
        return;
      }

      activeConversationController?.replaceEvents((current) => [
        ...current.map((event) =>
          event.turnId === turnId &&
          event.payload.type === "turn_changes_recorded"
            ? {
                ...event,
                payload: {
                  ...event.payload,
                  change_set: result.changeSet,
                },
              }
            : event,
        ),
        {
          id: `local-turn-undo-${Date.now()}`,
          threadId,
          turnId,
          seq: Number.MAX_SAFE_INTEGER,
          createdAt: new Date().toISOString(),
          payload: {
            type: "turn_undo_completed",
            target_turn_id: turnId,
            files_changed: result.filesChanged,
          },
        },
      ]);
      setTurnUndoDialog(null);
      setFilePreview(null);
      await refreshWorkbench();
    } catch (error) {
      setTurnUndoDialog((current) =>
        current?.turnId === turnId
          ? {
              ...current,
              applying: false,
              error: error instanceof Error ? error.message : String(error),
            }
          : current,
      );
    }
  }

  async function applyDiffHunk(
    hunk: WorkspaceDiffHunk,
    action: WorkspaceDiffHunkAction,
  ) {
    if (!client || !activeThread || hunkActionKey) return;
    if (
      action === "discard" &&
      !window.confirm(
        `Discard this unstaged hunk from ${hunk.path}? This cannot be undone by OpenTopia.`,
      )
    ) {
      return;
    }

    const actionKey = `${action}:${hunk.scope}:${hunk.path}:${hunk.header}`;
    setHunkActionKey(actionKey);
    setWorkbenchError(null);
    try {
      const result = await client.applyWorkspaceDiffHunk(
        activeThread.id,
        hunk,
        action,
        true,
      );
      setWorkspaceDiff(result.diff);
      setFilePreview(null);
      activeConversationController?.appendLocalEvent({
        id: `local-hunk-${Date.now()}`,
        threadId: activeThread.id,
        turnId: null,
        seq: Number.MAX_SAFE_INTEGER,
        createdAt: new Date().toISOString(),
        payload: {
          type: "file_changed",
          path: result.path,
          summary: `${diffHunkActionLabel(action)} one diff hunk.`,
        },
      });
    } catch (error) {
      setWorkbenchError(error instanceof Error ? error.message : String(error));
    } finally {
      setHunkActionKey(null);
    }
  }

  // The failure reason is returned rather than only pushed into `serverError`,
  // so the settings panel can tell the user what actually went wrong instead of
  // guessing that the keyring was at fault.
  async function storeProviderApiKey(
    providerId: string,
    value: string,
  ): Promise<ProviderSecretOutcome> {
    if (isSavingSecret) {
      return { stored: false, error: "另一个密钥操作正在进行中，请稍后重试。" };
    }
    setIsSavingSecret(true);
    setServerError(null);
    try {
      const metadata = await setProviderApiKey(providerId, value);
      setSecretSources(await listSecretSources());
      return { stored: true, metadata };
    } catch (error) {
      const message = errorMessage(error);
      setServerError(message);
      return { stored: false, error: message };
    } finally {
      setIsSavingSecret(false);
    }
  }

  async function removeProviderApiKey(
    providerId: string,
  ): Promise<ProviderSecretOutcome> {
    if (isSavingSecret) {
      return { stored: false, error: "另一个密钥操作正在进行中，请稍后重试。" };
    }
    setIsSavingSecret(true);
    setServerError(null);
    try {
      const metadata = await deleteProviderApiKey(providerId);
      setSecretSources(await listSecretSources());
      return { stored: true, metadata };
    } catch (error) {
      const message = errorMessage(error);
      setServerError(message);
      return { stored: false, error: message };
    } finally {
      setIsSavingSecret(false);
    }
  }

  async function startCodexLogin(): Promise<CodexLoginStart | null> {
    if (!client) return null;
    try {
      const login = await client.startCodexLogin();
      setCodexAccount((current) => ({
        ...(current ?? {
          loggedIn: false,
          loginPending: true,
        }),
        loginPending: true,
        loginId: login.loginId,
        loginType: login.loginType,
        authUrl: login.authUrl,
        verificationUrl: login.verificationUrl,
        userCode: login.userCode,
      }));
      setCodexAccountError(null);
      return login;
    } catch (error) {
      const message = errorMessage(error);
      setCodexAccountError(message);
      return null;
    }
  }

  async function cancelCodexLogin(): Promise<void> {
    if (!client) return;
    try {
      await client.cancelCodexLogin();
      await refreshCodexAccount();
    } catch (error) {
      setCodexAccountError(errorMessage(error));
    }
  }

  async function logoutCodexAccount(): Promise<void> {
    if (!client) return;
    try {
      await client.logoutCodexAccount();
      await refreshCodexAccount();
    } catch (error) {
      setCodexAccountError(errorMessage(error));
    }
  }

  async function testProviderConnection(
    providerId: string,
    providerDrafts?: ProviderSettings[],
  ) {
    if (!client || providerTest?.status === "testing") return;
    setProviderTest({ providerId, status: "testing" });
    try {
      if (providerDrafts) {
        const updated = await client.updateSettings({
          providers: providerDrafts,
        });
        setSettings(updated);
      }
      const result = await client.testProviderConnection(providerId);
      const updated = await client.getSettings();
      setSettings(updated);
      setProviderHealth(await client.getProviderHealth());
      setProviderTest({ providerId, status: "complete", result });
    } catch (error) {
      setProviderTest({
        providerId,
        status: "complete",
        result: {
          reachable: false,
          modelAvailable: false,
          error: friendlyProviderError(
            error instanceof Error ? error.message : String(error),
          ),
        },
      });
    }
  }

  function commitWorkspacePanelSize(
    key: keyof WorkspaceLayoutPreferences,
    value: number,
  ) {
    setWorkspaceLayoutPreferences((current) =>
      current[key] === value ? current : { ...current, [key]: value },
    );
  }

  function scheduleWorkspacePanelSize(
    key: keyof WorkspaceLayoutPreferences,
    value: number,
  ) {
    pendingWorkspaceSizeRef.current = { key, value };
    if (workspaceResizeFrameRef.current !== null) return;
    workspaceResizeFrameRef.current = window.requestAnimationFrame(() => {
      workspaceResizeFrameRef.current = null;
      const pending = pendingWorkspaceSizeRef.current;
      pendingWorkspaceSizeRef.current = null;
      if (pending) commitWorkspacePanelSize(pending.key, pending.value);
    });
  }

  function flushWorkspacePanelSize() {
    if (workspaceResizeFrameRef.current !== null) {
      window.cancelAnimationFrame(workspaceResizeFrameRef.current);
      workspaceResizeFrameRef.current = null;
    }
    const pending = pendingWorkspaceSizeRef.current;
    pendingWorkspaceSizeRef.current = null;
    if (pending) commitWorkspacePanelSize(pending.key, pending.value);
  }

  function beginWorkspaceResize(
    side: WorkspaceResizeSide,
    event: ReactPointerEvent<HTMLDivElement>,
  ) {
    if (event.button !== 0 || !event.isPrimary) return;
    event.preventDefault();
    const isLeft = side === "left";
    workspaceResizeDragRef.current = {
      side,
      preferenceKey: isLeft ? "left" : rightResizePreferenceKey,
      pointerId: event.pointerId,
      startX: event.clientX,
      startSize: isLeft ? workspaceLayout.left : workspaceLayout.right,
      latestSize: isLeft ? workspaceLayout.left : workspaceLayout.right,
      min: isLeft ? workspaceLayout.leftMin : workspaceLayout.rightMin,
      max: isLeft ? workspaceLayout.leftMax : workspaceLayout.rightMax,
    };
    setWorkspaceResizeSide(side);
    event.currentTarget.setPointerCapture(event.pointerId);
  }

  function continueWorkspaceResize(
    side: WorkspaceResizeSide,
    event: ReactPointerEvent<HTMLDivElement>,
  ) {
    const drag = workspaceResizeDragRef.current;
    if (!drag || drag.side !== side || drag.pointerId !== event.pointerId)
      return;
    event.preventDefault();
    const delta = event.clientX - drag.startX;
    const nextSize = clampPanelSize(
      drag.startSize + (side === "left" ? delta : -delta),
      drag.min,
      drag.max,
    );
    drag.latestSize = nextSize;
    scheduleWorkspacePanelSize(drag.preferenceKey, nextSize);
  }

  function finishWorkspaceResize(
    side: WorkspaceResizeSide,
    event: ReactPointerEvent<HTMLDivElement>,
  ) {
    const drag = workspaceResizeDragRef.current;
    if (!drag || drag.side !== side || drag.pointerId !== event.pointerId)
      return;
    scheduleWorkspacePanelSize(drag.preferenceKey, drag.latestSize);
    flushWorkspacePanelSize();
    workspaceResizeDragRef.current = null;
    setWorkspaceResizeSide(null);
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  }

  function resizeWorkspaceWithKeyboard(
    side: WorkspaceResizeSide,
    event: ReactKeyboardEvent<HTMLDivElement>,
  ) {
    const isLeft = side === "left";
    const current = isLeft ? workspaceLayout.left : workspaceLayout.right;
    const min = isLeft ? workspaceLayout.leftMin : workspaceLayout.rightMin;
    const max = isLeft ? workspaceLayout.leftMax : workspaceLayout.rightMax;
    const step = event.shiftKey ? 48 : 16;
    let next: number | null = null;

    if (event.key === "Home") next = min;
    else if (event.key === "End") next = max;
    else if (event.key === "ArrowLeft")
      next = current + (isLeft ? -step : step);
    else if (event.key === "ArrowRight")
      next = current + (isLeft ? step : -step);
    if (next === null) return;

    event.preventDefault();
    commitWorkspacePanelSize(
      isLeft ? "left" : rightResizePreferenceKey,
      clampPanelSize(next, min, max),
    );
  }

  function resetWorkspacePanelSize(side: WorkspaceResizeSide) {
    const key: keyof WorkspaceLayoutPreferences =
      side === "left" ? "left" : rightResizePreferenceKey;
    setWorkspaceLayoutPreferences((current) => {
      if (current[key] === undefined) return current;
      const next = { ...current };
      delete next[key];
      return next;
    });
  }

  async function syncProviderModels(
    providerId: string,
  ): Promise<ProviderModelSyncResult> {
    if (!client) {
      throw new Error("无法连接到 OpenTopia 后端，无法识别模型。");
    }
    try {
      const result = await client.syncProviderModels(providerId);
      setSettings((current) =>
        current
          ? {
              ...current,
              providers: current.providers.map((provider) =>
                provider.id === providerId ? result.provider : provider,
              ),
            }
          : current,
      );
      return result;
    } catch (error) {
      console.error("OpenTopia model discovery failed", {
        providerId,
        error: errorMessage(error),
      });
      throw error;
    }
  }

  function changeModelSelection(selection: ThreadModelSelection) {
    if (!settings || activeTurnId) return;

    // Follow the picked connection globally so new threads and utility calls
    // (title generation, guardian) land on the same API the user just chose.
    if (
      settings.activeProviderId !== selection.connectionId &&
      !isSavingSettings
    ) {
      void saveSettings({ activeProviderId: selection.connectionId });
    }

    // Keep the new-task composer on the last model state chosen by the user,
    // including selections made while an existing thread is open.
    setDraftModelSelection(selection);

    if (!activeThreadId) {
      return;
    }

    const threadId = activeThreadId;
    setThreads((current) =>
      current.map((thread) =>
        thread.id === threadId
          ? { ...thread, modelSelection: selection }
          : thread,
      ),
    );
    void (async () => {
      try {
        const updated = await client?.setThreadModel(threadId, selection);
        if (updated) {
          setThreads((current) =>
            current.map((thread) =>
              thread.id === threadId ? updated : thread,
            ),
          );
        }
      } catch (error) {
        setActionError(`切换模型失败：${errorMessage(error)}`);
      }
    })();
  }

  const closeTaskSearch = useCallback(() => setTaskSearchOpen(false), []);

  const showWindowsSandboxSetupPrompt = shouldPromptForWindowsSandboxSetup({
    isWindows,
    status: windowsSandboxSetup,
    dismissedForLaunch: windowsSandboxPromptDismissed,
  });

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing) return;
      if (!(event.ctrlKey || event.metaKey)) return;

      const key = event.key.toLocaleLowerCase();
      if (!event.altKey && !event.shiftKey && key === "w") {
        event.preventDefault();
        void closeAppWindow();
        return;
      }
      if (!event.altKey && !event.shiftKey && key === "q") {
        event.preventDefault();
        void quitApp();
        return;
      }
      if (taskSearchOpen) return;
      if (key === ",") {
        event.preventDefault();
        if (!settingsOpen) openSettings();
        return;
      }
      if (
        showWindowsSandboxSetupPrompt ||
        settingsOpen ||
        keyboardShortcutsOpen ||
        aboutOpen ||
        logViewerOpen ||
        renameTarget ||
        turnUndoDialog
      ) {
        return;
      }

      if (key === "k" && !event.altKey && !event.shiftKey) {
        event.preventDefault();
        setTaskSearchOpen(true);
        return;
      }
      if (event.altKey) {
        if (key === "b" && !event.shiftKey) {
          event.preventDefault();
          toggleToolPanel("diff");
        } else if (key === "s" && !event.shiftKey && activeThread) {
          event.preventDefault();
          void openSideTask();
        }
        return;
      }

      if (key === "b" && !event.shiftKey) {
        event.preventDefault();
        setSidebarCollapsed((current) => !current);
      } else if (key === "n" && !event.shiftKey) {
        event.preventDefault();
        beginNewThread();
      } else if (key === "o" && !event.shiftKey) {
        event.preventDefault();
        void chooseWorkspace();
      } else if (key === "`" && !event.shiftKey) {
        event.preventDefault();
        toggleToolPanel("terminal");
      } else if (key === "t" && !event.shiftKey) {
        event.preventDefault();
        openNewBrowserTab();
      } else if (key === "p" && !event.shiftKey) {
        event.preventDefault();
        toggleToolPanel("files");
      } else if (key === "e" && event.shiftKey) {
        event.preventDefault();
        toggleToolPanel("files");
      } else if (key === "/" && !event.shiftKey) {
        event.preventDefault();
        setKeyboardShortcutsOpen(true);
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  });

  return (
    <WorkspacePathIndexContext.Provider value={workspacePathIndex}>
      <div className="app-shell">
        <TopBar
          sidebarCollapsed={sidebarCollapsed}
          onToggleSidebar={() => setSidebarCollapsed((current) => !current)}
          onNewWindow={() => void newAppWindow()}
          onNewChat={beginNewThread}
          onOpenWorkspace={() => void chooseWorkspace()}
          onCloseWindow={() => void closeAppWindow()}
          onLogout={() => void logoutCodexAccount()}
          onQuit={() => void quitApp()}
          onToggleTool={toggleToolPanel}
          onOpenSettings={openSettings}
          onOpenLogs={() => setLogViewerOpen(true)}
          onShowKeyboardShortcuts={() => setKeyboardShortcutsOpen(true)}
          onShowAbout={() => setAboutOpen(true)}
          menuSuppressed={
            showWindowsSandboxSetupPrompt ||
            settingsOpen ||
            keyboardShortcutsOpen ||
            aboutOpen ||
            taskSearchOpen
          }
        />
        {actionError && (
          <div className="action-error" role="alert">
            <span>{actionError}</span>
            <button
              type="button"
              title="关闭"
              aria-label="关闭错误提示"
              onClick={() => setActionError(null)}
            >
              <X size={14} />
            </button>
          </div>
        )}
        <main
          ref={workspaceRef}
          className={`workspace ${settingsOpen ? "is-settings-hidden" : ""} ${toolStageOpen ? "with-tool-stage" : ""} ${flowInspectorOpen ? "with-flow-inspector" : ""} ${toolStageCoversConversation ? "tool-only" : ""} ${sidebarCollapsed ? "sidebar-collapsed" : ""} ${workspaceResizeSide ? "is-resizing" : ""}`}
          style={workspaceStyle}
        >
          <FlowWorkspaceProvider>
            <Sidebar
              projects={projects}
              threads={threads}
              threadActivityStore={threadActivityStore}
              activeThreadId={
                sidebarDestination === "conversation" ? activeThreadId : null
              }
              activeProjectId={
                sidebarDestination === "conversation"
                  ? (activeThread?.projectId ?? draftProjectId)
                  : null
              }
              workspaceError={workspaceError}
              isPickingWorkspace={isPickingWorkspace}
              experienceMode={experienceMode}
              flowModeEnabled={settings?.enterprise.enabled ?? false}
              newTaskOpen={
                sidebarDestination === "conversation" && activeThreadId === null
              }
              activeFlowPrimaryView={resolveActiveFlowPrimaryView({
                flowPrimaryView,
                sidebarDestination,
              })}
              pluginsOpen={sidebarDestination === "plugins"}
              contextualCollection={
                sidebarDestination === "flow-connections" && client ? (
                  <ConnectionSidebarCollection client={client} />
                ) : flowPrimarySurface && client ? (
                  <EnterpriseSidebarCollection
                    client={client}
                    view={flowPrimaryView}
                  />
                ) : undefined
              }
              onExperienceModeChange={changeExperienceMode}
              onOpenFlowPrimaryView={navigateFlowPrimaryView}
              onSelect={selectThread}
              onNew={beginNewThread}
              onPickWorkspace={() => void chooseWorkspace()}
              onCreateProject={createBlankProject}
              onRemoveProject={(project) => void removeProject(project)}
              onRenameProject={(project) =>
                setRenameTarget({
                  kind: "project",
                  id: project.id,
                  name: project.name,
                })
              }
              onToggleProjectPinned={(project) =>
                void toggleProjectPinned(project)
              }
              onSelectProject={beginProjectDraft}
              onNewThreadForProject={handleNewThreadForProject}
              onRenameThread={(thread) =>
                setRenameTarget({
                  kind: "thread",
                  id: thread.id,
                  name: thread.title,
                })
              }
              onOpenThreadUsage={(thread) => {
                selectThread(thread.id);
                openToolTab("usage");
              }}
              onArchiveThread={(thread) => void archiveThread(thread)}
              onRestoreThread={(thread) => void restoreThread(thread)}
              onOpenThreadWorkspace={(workspaceRoot) =>
                void openWorkspaceRoot(workspaceRoot)
              }
              onOpenExtensions={() => openToolTab("extensions")}
              onOpenTaskSearch={() => setTaskSearchOpen(true)}
              onSettings={openSettings}
            />
            {!settingsOpen && !sidebarCollapsed ? (
              <div
                className={`workspace-resizer workspace-resizer-left ${workspaceResizeSide === "left" ? "active" : ""}`}
                role="separator"
                tabIndex={0}
                aria-label="调整左侧栏宽度"
                aria-controls="workspace-sidebar"
                aria-orientation="vertical"
                aria-valuemin={workspaceLayout.leftMin}
                aria-valuemax={workspaceLayout.leftMax}
                aria-valuenow={workspaceLayout.left}
                aria-valuetext={`${workspaceLayout.left} 像素`}
                onPointerDown={(event) => beginWorkspaceResize("left", event)}
                onPointerMove={(event) =>
                  continueWorkspaceResize("left", event)
                }
                onPointerUp={(event) => finishWorkspaceResize("left", event)}
                onPointerCancel={(event) =>
                  finishWorkspaceResize("left", event)
                }
                onLostPointerCapture={(event) =>
                  finishWorkspaceResize("left", event)
                }
                onDoubleClick={() => resetWorkspacePanelSize("left")}
                onKeyDown={(event) =>
                  resizeWorkspaceWithKeyboard("left", event)
                }
              />
            ) : null}
            <section
              className={`center-pane ${
                activeApproval
                  ? "has-approval"
                  : activeUserInput
                    ? "has-plan-choice"
                    : ""
              } ${conversationFileDrop.isDraggingFiles ? "is-dragging-files" : ""}`}
              id="workspace-center-pane"
              onDragEnter={conversationFileDrop.onDragEnter}
              onDragOver={conversationFileDrop.onDragOver}
              onDragLeave={conversationFileDrop.onDragLeave}
              onDrop={conversationFileDrop.onDrop}
            >
              <FlowWorkspaceTitle
                fallback={flowPrimaryHeadingTitle(flowPrimaryView)}
              >
                {(workspaceTitle) => (
                  <ThreadHeader
                    thread={flowPrimarySurface ? null : activeThread}
                    backLabel={flowPageHeader?.backLabel}
                    headingIcon={
                      flowPrimarySurface
                        ? flowPrimaryHeadingIcon(flowPrimaryView)
                        : undefined
                    }
                    title={
                      flowPrimarySurface
                        ? (flowPageHeader?.title ?? workspaceTitle)
                        : undefined
                    }
                    onBack={
                      flowPrimarySurface ? flowPageHeader?.onBack : undefined
                    }
                    showThreadControls={!flowPrimarySurface}
                    toolStageOpen={toolStageOpen}
                    contextRailOpen={contextRailVisible}
                    onOpenLocation={() =>
                      activeThread &&
                      void openWorkspaceRoot(activeThread.workspaceRoot)
                    }
                    onOpenTool={openToolTab}
                    onToggleContextRail={() => {
                      if (toolStageOpen) {
                        setToolStageOpen(false);
                        setContextRailCollapsed(false);
                        setContextRailOpen(true);
                        return;
                      }
                      if (contextRailAutoVisible) {
                        setContextRailOpen(false);
                        setContextRailCollapsed((current) => !current);
                        return;
                      }
                      setContextRailCollapsed(false);
                      setContextRailOpen((current) => !current);
                    }}
                    onToggleToolStage={() => {
                      setConversationCollapsed(false);
                      if (
                        !toolStageOpen &&
                        activeThread?.experienceMode === "flow"
                      ) {
                        openToolTab("flow");
                        return;
                      }
                      setToolStageOpen((current) => !current);
                    }}
                    onRename={() =>
                      activeThread &&
                      setRenameTarget({
                        kind: "thread",
                        id: activeThread.id,
                        name: activeThread.title,
                      })
                    }
                    onArchive={() =>
                      activeThread && void archiveThread(activeThread)
                    }
                  />
                )}
              </FlowWorkspaceTitle>
              {conversationFileDrop.isDraggingFiles ? (
                <ConversationFileDropTarget />
              ) : null}
              {flowPrimarySurface && client ? (
                <FlowEnterpriseWorkspace
                  client={client}
                  onNavigate={navigateFlowPrimaryView}
                  onPageHeaderChange={setFlowPageHeader}
                  settings={settings}
                  threadId={
                    activeThread?.experienceMode === "flow"
                      ? activeThread.id
                      : null
                  }
                  view={
                    flowPrimaryView as Exclude<FlowPrimaryView, "conversation">
                  }
                  workspaceRoot={currentWorkspaceRoot}
                />
              ) : serverStatus === "offline" ? (
                <OfflineState
                  backendUrl={platform?.backendUrl}
                  error={serverError}
                  isProbing={serverProbing}
                  startupStatus={backendStartupStatus}
                  onRetry={() => {
                    setServerError(null);
                    setBootstrapRetryNonce((nonce) => nonce + 1);
                  }}
                />
              ) : activeThread ? (
                <>
                  {conversationGoalSnapshot ? (
                    <GoalStrip
                      snapshot={conversationGoalSnapshot}
                      isRunning={Boolean(conversationActiveTurnId)}
                      action={goalAction}
                      onRun={() => void runGoal()}
                      onPause={() => void changeGoalStatus("paused")}
                      onCancel={() => void changeGoalStatus("cancelled")}
                    />
                  ) : null}
                  {conversationLoadError ? (
                    <ConversationLoadErrorState
                      error={conversationLoadError}
                      onRetry={() => activeConversationController?.retry()}
                    />
                  ) : isConversationLoading ? (
                    <ConversationLoadingState />
                  ) : (
                    <LiveConversationMessageList
                      key={activeThread.id}
                      conversationRegistry={conversationRegistry!}
                      onEventsCommitted={commitConversationEventTraces}
                      undoingTurnId={
                        turnUndoDialog?.loading || turnUndoDialog?.applying
                          ? turnUndoDialog.turnId
                          : null
                      }
                      threadId={activeThread.id}
                      artifacts={artifacts}
                      onOpenArtifact={(artifactId) =>
                        void openArtifact(activeThread.id, artifactId)
                      }
                      onOpenImagePreview={(sourceId, image) =>
                        openInlineImagePreview(activeThread.id, sourceId, image)
                      }
                      onOpenAttachmentPreview={(source) =>
                        openPreviewTab(
                          activeThread.id,
                          { type: "attachment", attachmentId: source.id },
                          source.name,
                        )
                      }
                      onOpenMarkdownLink={openMarkdownLink}
                      onImplementProposedPlan={() => {
                        setCollaborationMode("default");
                        void submitMessage(
                          "请按上面的方案开始实施。",
                          [],
                          [],
                          "default",
                        );
                      }}
                      isProposedPlanActionDisabled={
                        isSending ||
                        Boolean(conversationActiveTurnId) ||
                        Boolean(activeApproval) ||
                        Boolean(activeUserInput)
                      }
                      onUndoTurn={(turnId) => void openTurnUndo(turnId)}
                      onReviewChanges={() => {
                        openToolTab("diff");
                        void refreshWorkbench();
                      }}
                      onOpenFileReview={openFileReview}
                      onLoadTurnFilePreview={(turnId, path, offset) => {
                        if (!client) {
                          return Promise.reject(new Error("服务尚未连接"));
                        }
                        return client.getTurnFileDiffPreview(
                          activeThread.id,
                          turnId,
                          path,
                          offset,
                        );
                      }}
                    />
                  )}
                  {activeApproval ? (
                    <ApprovalDialog
                      key={activeApproval.approval_id}
                      request={activeApproval}
                      queuePosition={1}
                      queueLength={pendingApprovalQueue.length}
                      isSubmitting={
                        decidingApprovalId === activeApproval.approval_id
                      }
                      error={approvalDecisionError}
                      onDecision={(approved) =>
                        void decideApproval(
                          activeApproval.approval_id,
                          approved,
                        )
                      }
                    />
                  ) : activeUserInput ? (
                    <PlanChoiceCard
                      key={activeUserInput.request.requestId}
                      request={activeUserInput.request}
                      isSubmitting={
                        submittingUserInputId ===
                        activeUserInput.request.requestId
                      }
                      error={userInputError}
                      onSubmit={(response) =>
                        void submitUserInput(
                          activeUserInput.request.requestId,
                          response,
                        )
                      }
                      onSkip={() =>
                        void submitUserInput(
                          activeUserInput.request.requestId,
                          {
                            answers: [],
                            skipped: true,
                          },
                        )
                      }
                      onCancel={() =>
                        void submitUserInput(
                          activeUserInput.request.requestId,
                          {
                            answers: [],
                            cancelled: true,
                          },
                        )
                      }
                    />
                  ) : (
                    <LiveConversationComposer
                      conversationRegistry={conversationRegistry!}
                      threadId={activeThread.id}
                      goalSnapshot={conversationGoalSnapshot}
                      fileDropHandleRef={conversationComposerFileDropHandle}
                      fileDropScope="conversation"
                      value={composer}
                      sendShortcut={editorPreferences.sendShortcut}
                      isSending={isSending}
                      isRunning={conversationTurnCanBeCancelled}
                      isCancelling={conversationTurnIsCancelling}
                      queuedMessageCount={
                        isConversationReady ? queuedMessageCount : 0
                      }
                      showContextWindowUsage={
                        editorPreferences.showContextWindowUsage
                      }
                      modelSelection={activeThread?.modelSelection ?? null}
                      providers={settings?.providers ?? []}
                      activeProviderId={settings?.activeProviderId ?? ""}
                      permissionMode={settings?.permissionMode ?? "auto"}
                      collaborationMode={collaborationMode}
                      sandboxMode={
                        settings?.sandbox.sandboxMode ?? "workspace-write"
                      }
                      contextSources={contextSources}
                      skills={skills}
                      selectedSkillIds={selectedSkillIds}
                      workspaceRoot={null}
                      projectName={null}
                      projects={projects}
                      onChange={setComposer}
                      onSubmit={submitMessage}
                      onCancel={() => void cancelTurn()}
                      onPickWorkspace={() => void chooseWorkspace()}
                      onSelectProject={selectProject}
                      onChangePermissionMode={changeExecutionPreset}
                      onChangeCollaborationMode={setCollaborationMode}
                      onChangeSandboxMode={changeSandboxMode}
                      onChangeModelSelection={changeModelSelection}
                      onOpenSettings={openModelSettings}
                      onAddContextSources={addContextSources}
                      onRemoveContextSource={removeContextSource}
                      onToggleSkill={toggleSkill}
                    />
                  )}
                </>
              ) : (
                <NewTaskState
                  value={composer}
                  sendShortcut={editorPreferences.sendShortcut}
                  workspaceRoot={currentWorkspaceRoot}
                  projectName={draftProject?.name ?? null}
                  projects={projects}
                  modelSelection={draftModelSelection}
                  providers={settings?.providers ?? []}
                  activeProviderId={settings?.activeProviderId ?? ""}
                  permissionMode={settings?.permissionMode ?? "auto"}
                  collaborationMode={collaborationMode}
                  sandboxMode={
                    settings?.sandbox.sandboxMode ?? "workspace-write"
                  }
                  contextSources={contextSources}
                  skills={skills}
                  selectedSkillIds={selectedSkillIds}
                  isSending={isSending}
                  launchMode={newTaskLaunchMode}
                  experienceMode={experienceMode}
                  onChange={setComposer}
                  onChangeLaunchMode={setNewTaskLaunchMode}
                  onPickWorkspace={() => void chooseWorkspace(true)}
                  onSelectProject={selectProject}
                  onChangePermissionMode={changeExecutionPreset}
                  onChangeCollaborationMode={setCollaborationMode}
                  onChangeSandboxMode={changeSandboxMode}
                  onChangeModelSelection={changeModelSelection}
                  onOpenSettings={openModelSettings}
                  onAddContextSources={addContextSources}
                  onRemoveContextSource={removeContextSource}
                  onToggleSkill={toggleSkill}
                  onSubmit={createThread}
                />
              )}
            </section>
            {(toolStageOpen || flowInspectorOpen) && !settingsOpen ? (
              <div
                className={`workspace-resizer workspace-resizer-right ${workspaceResizeSide === "right" ? "active" : ""}`}
                role="separator"
                tabIndex={0}
                aria-label="调整右侧栏宽度"
                aria-controls="workspace-right-panel"
                aria-orientation="vertical"
                aria-valuemin={workspaceLayout.rightMin}
                aria-valuemax={workspaceLayout.rightMax}
                aria-valuenow={workspaceLayout.right}
                aria-valuetext={`${workspaceLayout.right} 像素`}
                onPointerDown={(event) => beginWorkspaceResize("right", event)}
                onPointerMove={(event) =>
                  continueWorkspaceResize("right", event)
                }
                onPointerUp={(event) => finishWorkspaceResize("right", event)}
                onPointerCancel={(event) =>
                  finishWorkspaceResize("right", event)
                }
                onLostPointerCapture={(event) =>
                  finishWorkspaceResize("right", event)
                }
                onDoubleClick={() => resetWorkspacePanelSize("right")}
                onKeyDown={(event) =>
                  resizeWorkspaceWithKeyboard("right", event)
                }
              />
            ) : null}
            <RightPanel
              client={client}
              conversationRegistry={conversationRegistry}
              experienceMode={experienceMode}
              flowInspectorOpen={flowInspectorOpen}
              threads={threads}
              toolTabs={toolTabs}
              activeToolTab={activeToolTab}
              toolStageOpen={toolStageOpen}
              conversationCollapsed={conversationCollapsed}
              activeToolRequiresFullWorkspace={activeToolRequiresFullWorkspace}
              contextRailOpen={contextRailVisible}
              contextRailAutoVisible={contextRailAutoVisible}
              thread={activeThread}
              settings={settings}
              projects={projects}
              skills={skills}
              collaborationMode={collaborationMode}
              sendShortcut={editorPreferences.sendShortcut}
              showContextWindowUsage={editorPreferences.showContextWindowUsage}
              libraryProvider={activeFlowLibraryProvider}
              workspaceRoot={currentWorkspaceRoot}
              conversationLoading={isConversationLoading}
              agentItems={isConversationReady ? agentItems : []}
              terminalEvents={terminalEvents}
              terminalSession={terminalSession}
              workspaceTree={workspaceTree}
              filePreview={filePreview}
              workspaceDiff={workspaceDiff}
              sandbox={sandbox}
              plugins={plugins}
              selectedSkillIds={selectedSkillIds}
              mcpServers={mcpServers}
              threadMcpServers={threadMcpServers}
              workbenchError={workbenchError}
              isRefreshingWorkbench={isRefreshingWorkbench}
              pendingApprovalIds={pendingApprovalIds}
              decidingApprovalId={decidingApprovalId}
              artifacts={artifacts}
              contextStatus={contextStatus}
              isCompactingContext={isCompactingContext}
              revertingDiffPath={revertingDiffPath}
              hunkActionKey={hunkActionKey}
              reviewFileRequest={reviewFileRequest}
              onDecideApproval={decideApproval}
              onRefreshWorkbench={() => void refreshWorkbench()}
              onOpenWorkspacePath={(path) => void openWorkspacePath(path)}
              onOpenWorkspaceEntry={(entry) => void openWorkspaceEntry(entry)}
              onToggleThreadMcp={(serverId, enabled) =>
                void toggleThreadMcp(serverId, enabled)
              }
              onCreateMcpServer={createMcpServer}
              onUpdateMcpServer={updateMcpServer}
              onRestartMcpServer={restartMcpServer}
              onDeleteMcpServer={deleteMcpServer}
              onInstallPlugin={installLocalPlugin}
              onUninstallPlugin={uninstallLocalPlugin}
              onToggleThreadPlugin={toggleThreadPlugin}
              onUsePluginSkills={usePluginSkills}
              onOpenWorkspace={(workspaceRoot) =>
                void openWorkspaceRoot(workspaceRoot)
              }
              onEnsureTerminalSession={ensureTerminalSession}
              onWriteTerminalSession={(threadId, sessionId, data) =>
                void writeTerminalSession(threadId, sessionId, data)
              }
              onResizeTerminalSession={(threadId, sessionId, cols, rows) =>
                void resizeTerminalSession(threadId, sessionId, cols, rows)
              }
              onCloseTerminalSession={(threadId, sessionId) =>
                void closeTerminalSession(threadId, sessionId)
              }
              onCompactContext={() => void compactContext()}
              onOpenArtifact={(threadId, artifactId) =>
                void openArtifact(threadId, artifactId)
              }
              onOpenPreview={openPreviewTab}
              onOpenMarkdownLink={openMarkdownLink}
              onRevertDiffFile={(path) => void revertDiffFile(path)}
              onApplyDiffHunk={(hunk, action) =>
                void applyDiffHunk(hunk, action)
              }
              onOpenFileTab={openReviewFileTab}
              onLoadFileContent={loadReviewFileContent}
              onLoadTurnFileDiff={loadReviewTurnFileDiff}
              onGitAction={runReviewGitAction}
              onGetArtifact={(threadId, artifactId) =>
                getArtifact(threadId, artifactId)
              }
              onOpenImagePreview={openInlineImagePreview}
              onOpenToolTab={openToolTab}
              onOpenNewBrowserTab={openNewBrowserTab}
              onBrowserTabStateChange={updateBrowserTabState}
              onOpenSideTask={() => void openSideTask()}
              onThreadUpdated={(updatedThread) =>
                setThreads((current) =>
                  current.map((thread) =>
                    thread.id === updatedThread.id ? updatedThread : thread,
                  ),
                )
              }
              onChangePermissionMode={changeExecutionPreset}
              onChangeSandboxMode={changeSandboxMode}
              onChangeLibraryProvider={changeFlowLibraryProvider}
              onOpenSettings={openModelSettings}
              onActivateToolTab={setActiveToolTabId}
              onCloseToolTab={closeToolTab}
              previewSessionStore={previewSessionStore}
              onToggleConversation={() =>
                setConversationCollapsed((current) => !current)
              }
              onHideToolStage={() => {
                setToolStageOpen(false);
                setConversationCollapsed(false);
              }}
              onAddContextSources={() => void addContextSources()}
              onInterruptAgent={(agentThreadId) =>
                void interruptAgent(agentThreadId)
              }
            />
          </FlowWorkspaceProvider>
        </main>
        {settingsOpen && (
          <RedesignedSettingsPanel
            client={client}
            initialTab={settingsInitialTab}
            platform={platform}
            settings={settings}
            providerHealth={providerHealth}
            codexAccount={codexAccount}
            codexAccountLoading={codexAccountLoading}
            codexAccountError={codexAccountError}
            providerTest={providerTest}
            secretSources={secretSources}
            notificationPreferences={taskNotificationPreferences}
            appearance={appearance}
            resolvedTheme={resolvedTheme}
            personalization={personalization}
            editorPreferences={editorPreferences}
            isSaving={isSavingSettings}
            isSavingSecret={isSavingSecret}
            sidebarResize={{
              width: workspaceLayout.left,
              minWidth: workspaceLayout.leftMin,
              maxWidth: workspaceLayout.leftMax,
              isResizing: workspaceResizeSide === "left",
              onPointerDown: (event) => beginWorkspaceResize("left", event),
              onPointerMove: (event) => continueWorkspaceResize("left", event),
              onPointerUp: (event) => finishWorkspaceResize("left", event),
              onPointerCancel: (event) => finishWorkspaceResize("left", event),
              onLostPointerCapture: (event) =>
                finishWorkspaceResize("left", event),
              onDoubleClick: () => resetWorkspacePanelSize("left"),
              onKeyDown: (event) => resizeWorkspaceWithKeyboard("left", event),
            }}
            onAppearanceChange={setAppearance}
            onPersonalizationChange={setPersonalization}
            onEditorPreferencesChange={setEditorPreferences}
            onSave={saveSettings}
            onTestProvider={(providerId, providers) =>
              void testProviderConnection(providerId, providers)
            }
            onSyncProviderModels={syncProviderModels}
            onStoreProviderApiKey={storeProviderApiKey}
            onDeleteProviderApiKey={removeProviderApiKey}
            onRefreshCodexAccount={() => void refreshCodexAccount()}
            onStartCodexLogin={startCodexLogin}
            onCancelCodexLogin={cancelCodexLogin}
            onLogoutCodexAccount={logoutCodexAccount}
            onNotificationPreferencesChange={setTaskNotificationPreferences}
            onTestNotification={() =>
              deliverTaskCompletionNotification(
                {
                  userMessage: "测试任务完成通知",
                  reply: "测试成功：OpenTopia 可以在任务完成时提醒你。",
                },
                true,
              )
            }
            windowsSandboxSetup={windowsSandboxSetup}
            windowsSandboxSetupBusy={windowsSandboxSetupBusy}
            windowsSandboxSetupError={windowsSandboxSetupError}
            onSetupWindowsSandbox={setupWindowsSandbox}
            onRemoveWindowsSandbox={removeWindowsSandbox}
            onOpenLogs={() => {
              closeSettings();
              setLogViewerOpen(true);
            }}
            onClose={closeSettings}
          />
        )}
        {showWindowsSandboxSetupPrompt && windowsSandboxSetup ? (
          <WindowsSandboxSetupDialog
            status={windowsSandboxSetup}
            busy={windowsSandboxSetupBusy}
            error={windowsSandboxSetupError}
            onSetup={() => {
              void setupWindowsSandbox().catch(() => undefined);
            }}
            onOpenSettings={() => {
              setWindowsSandboxPromptDismissed(true);
              openPermissionSettings();
            }}
            onLater={() => setWindowsSandboxPromptDismissed(true)}
          />
        ) : null}
        {taskSearchOpen ? (
          <TaskSearchDialog
            activeThreadId={activeThreadId}
            activityStore={threadActivityStore}
            projects={projects}
            threads={threads}
            onClose={closeTaskSearch}
            onSelectThread={selectThread}
          />
        ) : null}
        {keyboardShortcutsOpen ? (
          <KeyboardShortcutsDialog
            onClose={() => setKeyboardShortcutsOpen(false)}
          />
        ) : null}
        {aboutOpen ? <AboutDialog onClose={() => setAboutOpen(false)} /> : null}
        {logViewerOpen && <LogViewer onClose={() => setLogViewerOpen(false)} />}
        {renameTarget && (
          <RenameDialog
            target={renameTarget}
            onSubmit={submitRename}
            onClose={() => setRenameTarget(null)}
          />
        )}
        {turnUndoDialog && (
          <TurnUndoDialog
            state={turnUndoDialog}
            onConfirm={() => void confirmTurnUndo()}
            onClose={() => {
              if (!turnUndoDialog.applying) setTurnUndoDialog(null);
            }}
          />
        )}
      </div>
    </WorkspacePathIndexContext.Provider>
  );
}

function diffHunkActionLabel(action: WorkspaceDiffHunkAction): string {
  switch (action) {
    case "stage":
      return "Staged";
    case "unstage":
      return "Unstaged";
    case "discard":
      return "Discarded";
  }
}

type LegacyLocalProject = {
  id: string;
  name: string;
};

const localProjectsStorageKey = "opentopia.localProjects";
const hiddenWorkspaceRootsStorageKey = "opentopia.hiddenWorkspaceRoots";
const projectApiMigrationStorageKey = "opentopia.projectApiMigration.v1";

function readLegacyLocalProjects(): LegacyLocalProject[] {
  try {
    const parsed = JSON.parse(
      window.localStorage.getItem(localProjectsStorageKey) ?? "[]",
    );
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((project): project is LegacyLocalProject =>
      Boolean(
        project &&
        typeof project.id === "string" &&
        typeof project.name === "string",
      ),
    );
  } catch {
    return [];
  }
}

function readLegacyHiddenWorkspaceRootKeys(): string[] {
  try {
    const parsed = JSON.parse(
      window.localStorage.getItem(hiddenWorkspaceRootsStorageKey) ?? "[]",
    );
    if (!Array.isArray(parsed)) return [];
    return [
      ...new Set(
        parsed
          .filter((key): key is string => typeof key === "string")
          .map(workspaceRootKey),
      ),
    ];
  } catch {
    return [];
  }
}

function projectApiMigrationComplete(): boolean {
  try {
    return window.localStorage.getItem(projectApiMigrationStorageKey) === "1";
  } catch {
    return false;
  }
}

function markProjectApiMigrationComplete() {
  try {
    window.localStorage.removeItem(localProjectsStorageKey);
    window.localStorage.removeItem(hiddenWorkspaceRootsStorageKey);
    window.localStorage.setItem(projectApiMigrationStorageKey, "1");
  } catch {
    // The API data remains authoritative when browser storage is unavailable.
  }
}

async function migrateLegacyProjectData(
  client: ApiClient,
  existingProjects: Project[],
  existingThreads: Thread[],
): Promise<{ projects: Project[]; threads: Thread[] }> {
  if (projectApiMigrationComplete()) {
    return { projects: existingProjects, threads: existingThreads };
  }

  const [recentWorkspaces, localProjects] = await Promise.all([
    getRecentWorkspaces(),
    Promise.resolve(readLegacyLocalProjects()),
  ]);
  const hiddenRoots = new Set(readLegacyHiddenWorkspaceRootKeys());
  const projects = [...existingProjects];
  const workspaceCandidates = new Map<
    string,
    Pick<RecentWorkspace, "name" | "workspaceRoot">
  >();

  for (const workspace of recentWorkspaces) {
    const key = workspaceRootKey(workspace.workspaceRoot);
    if (!hiddenRoots.has(key) && !workspaceCandidates.has(key)) {
      workspaceCandidates.set(key, workspace);
    }
  }
  for (const thread of existingThreads) {
    const key = workspaceRootKey(thread.workspaceRoot);
    if (!hiddenRoots.has(key) && !workspaceCandidates.has(key)) {
      workspaceCandidates.set(key, {
        name: workspaceName(thread.workspaceRoot),
        workspaceRoot: thread.workspaceRoot,
      });
    }
  }

  for (const candidate of workspaceCandidates.values()) {
    const key = workspaceRootKey(candidate.workspaceRoot);
    if (
      projects.some(
        (project) =>
          project.workspaceRoot &&
          workspaceRootKey(project.workspaceRoot) === key,
      )
    ) {
      continue;
    }
    projects.push(
      await client.createProject({
        name: candidate.name,
        workspaceRoot: candidate.workspaceRoot,
      }),
    );
  }

  for (const localProject of localProjects) {
    const duplicate = projects.some(
      (project) =>
        project.workspaceRoot === null &&
        project.name.trim().toLocaleLowerCase() ===
          localProject.name.trim().toLocaleLowerCase(),
    );
    if (!duplicate) {
      projects.push(await client.createProject({ name: localProject.name }));
    }
  }

  for (const thread of existingThreads) {
    if (thread.projectId) continue;
    const project = projects.find(
      (item) =>
        item.workspaceRoot &&
        workspaceRootKey(item.workspaceRoot) ===
          workspaceRootKey(thread.workspaceRoot),
    );
    if (project) {
      await client.updateThread(thread.id, { projectId: project.id });
    }
  }

  const [migratedProjects, migratedThreads] = await Promise.all([
    client.listProjects(),
    client.listThreads(),
  ]);
  try {
    await window.opentopia?.clearRecentWorkspaces();
  } catch (error) {
    console.warn("OpenTopia could not clear migrated recent workspaces", error);
  }
  markProjectApiMigrationComplete();
  return { projects: migratedProjects, threads: migratedThreads };
}

function sortProjects(projects: Project[]): Project[] {
  return [...projects].sort(
    (left, right) =>
      Number(right.pinned) - Number(left.pinned) ||
      left.sortOrder - right.sortOrder ||
      left.createdAt.localeCompare(right.createdAt),
  );
}

function controlledSandboxSettings(
  sandbox: AppSettings["sandbox"],
): AppSettings["sandbox"] {
  return {
    ...sandbox,
    sandboxMode: "workspace-write",
    enforcement: "enforce",
    network: sandbox.network,
  };
}

function parseDirectToolCommand(value: string): DirectToolCommand | null {
  const trimmed = value.trim();
  const match = /^\/(run|read)(?:\s+([\s\S]*))?$/i.exec(trimmed);
  if (!match) return null;

  const argument = match[2]?.trim();
  if (!argument) return null;
  return match[1].toLowerCase() === "run"
    ? { kind: "run", command: argument }
    : { kind: "read", path: argument };
}

function isLegacyDirectToolCommand(value: string): boolean {
  return /^\/(?:run|read)(?:\s|$)/i.test(value.trim());
}

function collectArtifactReferences(
  metadata: unknown,
  output?: string,
): ArtifactReference[] {
  return uniqueArtifactReferences([
    ...artifactReferencesFromMetadata(metadata),
    ...artifactReferencesFromText(output ?? ""),
  ]);
}

function artifactReferencesFromMetadata(
  metadata: unknown,
): ArtifactReference[] {
  if (!isRecord(metadata)) return [];
  const refs: ArtifactReference[] = [];
  const artifactId = readString(metadata.artifactId);
  if (artifactId) {
    refs.push({
      id: artifactId,
      kind: readString(metadata.artifactKind),
      contentType: readString(metadata.artifactContentType),
      bytes: readNumber(metadata.artifactBytes),
    });
  }
  if (isRecord(metadata.artifact)) {
    const nestedId = readString(metadata.artifact.id);
    if (nestedId) {
      refs.push({
        id: nestedId,
        kind: readString(metadata.artifact.kind),
        contentType: readString(metadata.artifact.contentType),
        bytes: readNumber(metadata.artifact.bytes),
        metadata: metadata.artifact.metadata,
      });
    }
  }
  if (Array.isArray(metadata.artifacts)) {
    for (const artifact of metadata.artifacts) {
      if (!isRecord(artifact)) continue;
      const id = readString(artifact.id);
      if (!id) continue;
      refs.push({
        id,
        kind: readString(artifact.kind),
        contentType: readString(artifact.contentType),
        bytes: readNumber(artifact.bytes),
        metadata: artifact.metadata,
      });
    }
  }
  return refs;
}

function uniqueArtifactReferences(
  refs: ArtifactReference[],
): ArtifactReference[] {
  const byId = new Map<string, ArtifactReference>();
  for (const ref of refs) {
    byId.set(ref.id, { ...byId.get(ref.id), ...ref });
  }
  return [...byId.values()];
}

function mergeArtifactDescriptors(
  current: ArtifactDescriptor[],
  refs: ArtifactReference[],
  event: AgentEvent,
): ArtifactDescriptor[] {
  let next = current;
  for (const ref of refs) {
    if (next.some((artifact) => artifact.id === ref.id)) continue;
    next = [
      ...next,
      {
        id: ref.id,
        threadId: event.threadId,
        kind: ref.kind ?? "tool_output",
        contentType: ref.contentType ?? "text/plain; charset=utf-8",
        bytes: ref.bytes ?? 0,
        createdAt: event.createdAt,
        metadata: ref.metadata,
      },
    ];
  }
  return next;
}

function flowPrimaryHeadingIcon(view: FlowPrimaryView) {
  if (view === "overview")
    return <LayoutDashboard aria-hidden="true" size={15} />;
  if (view === "agents") return <Bot aria-hidden="true" size={15} />;
  if (view === "workflow-templates")
    return <Workflow aria-hidden="true" size={15} />;
  if (view === "inbox") return <Inbox aria-hidden="true" size={15} />;
  if (view === "runs") return <Activity aria-hidden="true" size={15} />;
  if (view === "connections") return <Cable aria-hidden="true" size={15} />;
  if (view === "trust") return <ShieldCheck aria-hidden="true" size={15} />;
  if (view === "knowledge") return <Library aria-hidden="true" size={15} />;
  return undefined;
}

function flowPrimaryHeadingTitle(view: FlowPrimaryView): string | undefined {
  if (view === "overview") return "Overview / 运行总览";
  if (view === "agents") return "Agents / Agent 配置";
  if (view === "workflow-templates") return "Flows / 工作流";
  if (view === "inbox") return "Inbox / 待处理";
  if (view === "runs") return "Runs / 运行追踪";
  if (view === "connections") return "Connections / 连接";
  if (view === "trust") return "Trust / 信任中心";
  if (view === "knowledge") return "Knowledge / 知识库";
  return undefined;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function readString(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value : undefined;
}

function readNumber(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value)
    ? value
    : undefined;
}

function artifactPreviewTitle(
  descriptor: ArtifactDescriptor | undefined,
  artifactId: string,
): string {
  if (descriptor?.storage && "path" in descriptor.storage) {
    const path = descriptor.storage.path;
    if (typeof path === "string") {
      return path.split(/[\\/]/).at(-1) || path;
    }
  }
  return descriptor?.kind || artifactId;
}

function markdownLinkTitle(path: string): string {
  const normalized = path.replaceAll("\\", "/");
  return normalized.split("/").filter(Boolean).at(-1) ?? path;
}

function usesFormatAwarePreview(path: string): boolean {
  return (
    isSpreadsheetFilePath(path) ||
    /\.(?:avif|bmp|gif|ico|jpe?g|pdf|png|svg|webp)$/i.test(path)
  );
}
