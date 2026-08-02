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
  type CSSProperties,
  type DragEvent as ReactDragEvent,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { createPortal } from "react-dom";
import {
  Activity,
  AlertCircle,
  Archive,
  ArrowLeft,
  ArrowRight,
  ArrowDown,
  ArrowUp,
  Bot,
  Box,
  BriefcaseBusiness,
  Check,
  CircleAlert,
  CirclePlus,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  Circle,
  CircleHelp,
  Cloud,
  Clock3,
  Code2,
  Download,
  ExternalLink,
  File,
  FileCode2,
  FileImage,
  FileJson,
  FileText,
  Folder,
  FolderOpen,
  GitBranch,
  GitPullRequest,
  GitFork,
  Globe2,
  Hand,
  Laptop,
  ListTodo,
  Loader2,
  Maximize2,
  Minimize2,
  Monitor,
  MessageCircle,
  MoreHorizontal,
  PanelRight,
  PanelRightClose,
  PanelRightOpen,
  PanelLeftClose,
  PanelLeftOpen,
  Paperclip,
  Pause,
  Pencil,
  Pin,
  Plug,
  Plus,
  Presentation,
  RotateCcw,
  Search,
  Settings,
  ShieldAlert,
  ShieldCheck,
  SlidersHorizontal,
  Square,
  SquarePen,
  Target,
  Table2,
  TerminalSquare,
  Trash2,
  Workflow,
  X,
  Zap,
  ZoomIn,
  ZoomOut,
} from "lucide-react";
import { ApiClient } from "./api/client";
import type { StreamHandle } from "./api/client";
import type {
  DiffReviewFileContent,
  DiffReviewGitAction,
} from "./components/DiffReviewPanel";
import { LogViewer } from "./components/LogViewer";
import { MarkdownContent } from "./components/MarkdownContent";
import { ModelSelector } from "./components/ModelSelector";
import {
  ApprovalDialog,
  type ApprovalRequest,
} from "./components/ApprovalDialog";
import { PlanChoiceCard } from "./components/PlanChoiceCard";
import { PreviewHost } from "./components/PreviewHost";
import { RightContextRail } from "./components/RightContextRail";
import { SettingsPanel as RedesignedSettingsPanel } from "./components/SettingsPanel";
import { TaskSearchDialog } from "./components/TaskSearchDialog";
import {
  PendingTurnStatus,
  TurnActivityTimeline,
  TurnChangeCard,
} from "./components/TurnActivityTimeline";
import { WebPreviewSurface } from "./components/WebPreviewSurface";
import { ComputerPanel } from "./components/ComputerPanel";
import { WorkbenchPanel, type WorkbenchTab } from "./components/WorkbenchPanel";
import { Button, IconButton, Popover } from "./components/ui";
import { normalizeCopiedText } from "./clipboardText";
import { isConversationScrollNearEnd } from "./conversationScroll";
import {
  conversationStreamEventTrace,
  rendererTraceTime,
  type ConversationStreamEventTrace,
} from "./conversationRenderTrace";
import { resolveMarkdownLink } from "./markdownLinks";
import {
  useWorkspacePathIndex,
  WorkspacePathIndexContext,
} from "./components/WorkspacePathProvider";
import {
  reconcileReasoningEffort,
  resolveDefaultModelId,
} from "./modelCatalog";
import { modelSupportsVision } from "./modelCapabilities";
import {
  appendImageUnderstandingContext,
  buildImageUnderstandingArguments,
  extractImageUnderstandingText,
  isImageUnderstandingMcpTool,
} from "./imageProcessing";
import {
  deleteProviderApiKey,
  getDroppedContextFiles,
  getRecentWorkspaces,
  listSecretSources,
  loadPlatformInfo,
  openExternal,
  openPath,
  recordConversationRenderTrace,
  selectContextFiles,
  selectPluginDirectory,
  selectWorkspace,
  setProviderApiKey,
  showSystemNotification,
} from "./platform";
import {
  playCompletionChime,
  readTaskNotificationPreferences,
  shouldDeliverTaskNotification,
  writeTaskNotificationPreferences,
} from "./taskNotifications";
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
import type { ActiveTurnPhase } from "./turnActivityStatus";
import {
  readEditorPreferences,
  writeEditorPreferences,
} from "./editorPreferences";
import type { TaskSearchActivityStatus } from "./taskSearch";
import type {
  AgentEvent,
  AppSettings,
  ArtifactContent,
  ArtifactDescriptor,
  BrowserNavigationRequest,
  CollaborationMode,
  CodexAccountStatus,
  CodexLoginStart,
  ContextStatus,
  ContextSourceFile,
  ExperienceMode,
  GoalSnapshot,
  GoalStatus,
  InlineImageAttachment,
  McpServerInput,
  McpServerView,
  McpToolDescriptor,
  Message,
  MessagePart,
  PlatformInfo,
  PluginView,
  Project,
  ProviderHealth,
  ProviderHealthCheckResult,
  ProviderKind,
  ProviderModelSyncResult,
  ProviderSecretOutcome,
  ProviderSettings,
  PreviewTarget,
  RecentWorkspace,
  ReviewFileRequest,
  SandboxDescriptor,
  SecretSources,
  SkillDescriptor,
  SubagentRun,
  TaskPlan,
  TerminalEvent,
  TerminalSession,
  Thread,
  ThreadMcpServerView,
  ThreadModelSelection,
  TurnStatus,
  TurnChangeSet,
  TurnFileChange,
  TurnFileDiffPreview,
  TurnUndoPreview,
  UserInputRecord,
  UserInputResponse,
  WorkspaceDiff,
  WorkspaceDiffHunk,
  WorkspaceDiffHunkAction,
  WorkspaceEntry,
  WorkspaceFilePreview,
  WorkspaceTree,
} from "./types";

type ServerStatus = "checking" | "online" | "offline";

type ThreadActivityStatus = TaskSearchActivityStatus;

type ConversationLoadState = {
  threadId: string | null;
  status: "idle" | "loading" | "ready" | "error";
  error: string | null;
};

type PendingTurnFeedback = {
  threadId: string;
  turnId: string | null;
  phase: ActiveTurnPhase;
  startedAt: string;
};

function resolveThreadActivityStatus(
  turnStatus: TurnStatus | null,
): ThreadActivityStatus | null {
  switch (turnStatus?.status) {
    case "running":
    case "cancelling":
      return "processing";
    case "waiting_approval":
      return "approval";
    case "waiting_user_action":
      return "user_action";
    case "succeeded":
      return "succeeded";
    case "failed":
      return "failed";
    default:
      return null;
  }
}

async function loadThreadActivityStatuses(
  client: ApiClient,
  threadList: Thread[],
): Promise<Record<string, ThreadActivityStatus>> {
  const entries = await Promise.all(
    threadList.map(async (thread) => {
      try {
        const turnStatus = await client.getTurnStatus(thread.id);
        const status = resolveThreadActivityStatus(turnStatus);
        return status ? ([thread.id, status] as const) : null;
      } catch {
        return null;
      }
    }),
  );

  return Object.fromEntries(
    entries.filter(
      (entry): entry is readonly [string, ThreadActivityStatus] =>
        entry !== null,
    ),
  );
}

type ToolTabKind =
  WorkbenchTab | "browser" | "computer" | "preview" | "side-task";

type ToolTab = {
  id: string;
  kind: ToolTabKind;
  title: string;
  sideTaskThreadId?: string;
  previewTarget?: PreviewTarget;
  browserNavigation?: BrowserNavigationRequest;
};

type DirectToolCommand =
  { kind: "run"; command: string } | { kind: "read"; path: string };

type ExecutionPermissionMode = "auto" | "approve" | "full_access";
type NewTaskLaunchMode = "local" | "new_worktree";

type WorkspaceResizeSide = "left" | "right";

type WorkspaceLayoutPreferences = {
  left?: number;
  contextRight?: number;
  toolRight?: number;
};

type WorkspaceLayout = {
  left: number;
  leftMin: number;
  leftMax: number;
  right: number;
  rightMin: number;
  rightMax: number;
};

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

type TurnUndoDialogState = {
  turnId: string;
  preview: TurnUndoPreview | null;
  loading: boolean;
  applying: boolean;
  error: string | null;
};

const workspaceLayoutStorageKey = "opentopia.workspace-layout.v1";
const experienceModeStorageKey = "opentopia.experience-mode.v1";
const collaborationModeStorageKey = "opentopia.collaboration-mode.v1";
const workspaceThreePaneBreakpoint = 1120;
const contextRailInlineMinWidth = 1120;
const workspaceLeftMin = 200;
const workspaceLeftMax = 420;

function readExperienceMode(): ExperienceMode {
  if (typeof window === "undefined") return "code";
  try {
    return window.localStorage.getItem(experienceModeStorageKey) === "work"
      ? "work"
      : "code";
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

function reusableGoalId(
  mode: CollaborationMode,
  snapshot: GoalSnapshot | null,
): string | undefined {
  if (!snapshot || mode === "default") return undefined;
  if (["completed", "cancelled", "failed"].includes(snapshot.goal.status)) {
    return undefined;
  }
  if (mode === "goal") return snapshot.goal.id;
  return ["draft", "ready", "paused", "blocked"].includes(snapshot.goal.status)
    ? snapshot.goal.id
    : undefined;
}

function goalSnapshotAsTaskPlan(snapshot: GoalSnapshot): TaskPlan | null {
  if (snapshot.tasks.length === 0) return null;
  return {
    planRevision: snapshot.goal.planRevision,
    goalId: snapshot.goal.id,
    steps: snapshot.tasks.map((task) => ({
      id: task.stepId,
      title: task.title,
      status:
        task.status === "running"
          ? "in_progress"
          : task.status === "succeeded"
            ? "completed"
            : task.status === "failed"
              ? "blocked"
              : task.status,
      statusReason: task.statusReason,
      dependencies: task.dependencies,
      acceptanceCriteria: task.acceptanceCriteria,
      evidence: task.evidence,
    })),
  };
}

function resolveComposerTaskPlan(
  events: AgentEvent[],
  snapshot: GoalSnapshot | null,
): TaskPlan | null {
  const latestPlanEvent = [...events]
    .sort((left, right) => right.seq - left.seq)
    .find((event) => event.payload.type === "plan_updated");
  const latestPlan =
    latestPlanEvent?.payload.type === "plan_updated"
      ? latestPlanEvent.payload.plan
      : null;
  const goalPlan = snapshot ? goalSnapshotAsTaskPlan(snapshot) : null;

  if (goalPlan && (!latestPlan || latestPlan.goalId === goalPlan.goalId)) {
    return goalPlan;
  }
  return latestPlan ?? goalPlan;
}

function readWorkspaceLayoutPreferences(): WorkspaceLayoutPreferences {
  if (typeof window === "undefined") return {};
  try {
    const parsed = JSON.parse(
      window.localStorage.getItem(workspaceLayoutStorageKey) ?? "{}",
    ) as Record<string, unknown>;
    return {
      left: validStoredPanelSize(parsed.left),
      contextRight: validStoredPanelSize(parsed.contextRight),
      toolRight: validStoredPanelSize(parsed.toolRight),
    };
  } catch {
    return {};
  }
}

function validStoredPanelSize(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) && value > 0
    ? value
    : undefined;
}

function clampPanelSize(value: number, min: number, max: number): number {
  return Math.round(Math.min(Math.max(value, min), max));
}

function defaultWorkspaceLeftWidth(
  workspaceWidth: number,
  toolOnly: boolean,
): number {
  if (workspaceWidth <= 840) return toolOnly ? 210 : 226;
  if (workspaceWidth <= workspaceThreePaneBreakpoint)
    return toolOnly ? 210 : 252;
  return toolOnly ? 220 : 264;
}

function resolveWorkspaceLayout(
  preferences: WorkspaceLayoutPreferences,
  workspaceWidth: number,
  hasToolStage: boolean,
  toolOnly: boolean,
): WorkspaceLayout {
  const width = Math.max(workspaceWidth, 760);
  const compact = width <= workspaceThreePaneBreakpoint || toolOnly;
  const compactMainMin = hasToolStage ? 560 : 440;
  const centerMin = hasToolStage ? 360 : 480;
  const rightMin = hasToolStage ? 360 : 240;
  const rightCap = hasToolStage ? 1200 : 520;
  const leftMax = Math.max(
    workspaceLeftMin,
    Math.min(
      workspaceLeftMax,
      width - (compact ? compactMainMin : centerMin + rightMin),
    ),
  );
  const left = clampPanelSize(
    preferences.left ?? defaultWorkspaceLeftWidth(width, toolOnly),
    workspaceLeftMin,
    leftMax,
  );
  const rightMax = Math.max(
    rightMin,
    Math.min(rightCap, width - left - centerMin),
  );
  const defaultRight = hasToolStage
    ? width - left - clampPanelSize(width * 0.31, centerMin, 600)
    : 286;
  const preferredRight = hasToolStage
    ? preferences.toolRight
    : preferences.contextRight;

  return {
    left,
    leftMin: workspaceLeftMin,
    leftMax,
    right: clampPanelSize(preferredRight ?? defaultRight, rightMin, rightMax),
    rightMin,
    rightMax,
  };
}

function useDismissiblePopover(open: boolean, onClose: () => void) {
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;

    const handlePointerDown = (event: PointerEvent) => {
      if (!containerRef.current?.contains(event.target as Node)) onClose();
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };

    document.addEventListener("pointerdown", handlePointerDown, true);
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("pointerdown", handlePointerDown, true);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [onClose, open]);

  return containerRef;
}

function emptyContextUsage(): ContextStatus["usage"] {
  return {
    modelRequests: 0,
    inputTokens: 0,
    cachedInputTokens: 0,
    cacheWriteTokens: 0,
    reasoningTokens: 0,
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
  const [serverStatus, setServerStatus] = useState<ServerStatus>("checking");
  const [serverError, setServerError] = useState<string | null>(null);
  const [bootstrapAttempt, setBootstrapAttempt] = useState(0);
  const [serverProbing, setServerProbing] = useState(true);
  const clientEndpointRef = useRef<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [projects, setProjects] = useState<Project[]>([]);
  const [threads, setThreads] = useState<Thread[]>([]);
  const [activeThreadId, setActiveThreadId] = useState<string | null>(null);
  // Model picked on the new-task screen, before a thread exists to pin it to.
  // Carried into the thread the draft creates.
  const [draftModelSelection, setDraftModelSelection] =
    useState<ThreadModelSelection | null>(null);
  const [experienceMode, setExperienceMode] =
    useState<ExperienceMode>(readExperienceMode);
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
  const [messages, setMessages] = useState<Message[]>([]);
  const [events, setEvents] = useState<AgentEvent[]>([]);
  const [conversationLoadState, setConversationLoadState] =
    useState<ConversationLoadState>({
      threadId: null,
      status: "idle",
      error: null,
    });
  const [conversationLoadAttempt, setConversationLoadAttempt] = useState(0);
  const [subagentRuns, setSubagentRuns] = useState<SubagentRun[]>([]);
  const [terminalEvents, setTerminalEvents] = useState<TerminalEvent[]>([]);
  const [terminalSession, setTerminalSession] =
    useState<TerminalSession | null>(null);
  const [composer, setComposer] = useState("");
  const [newTaskLaunchMode, setNewTaskLaunchMode] =
    useState<NewTaskLaunchMode>("local");
  const [contextSources, setContextSources] = useState<ContextSourceFile[]>([]);
  const [skills, setSkills] = useState<SkillDescriptor[]>([]);
  const [selectedSkillIds, setSelectedSkillIds] = useState<string[]>([]);
  const [skillsRevision, setSkillsRevision] = useState(0);
  const [isSending, setIsSending] = useState(false);
  const [activeTurnId, setActiveTurnId] = useState<string | null>(null);
  const [pendingTurnFeedback, setPendingTurnFeedback] =
    useState<PendingTurnFeedback | null>(null);
  const [threadActivityStatuses, setThreadActivityStatuses] = useState<
    Record<string, ThreadActivityStatus>
  >({});
  const [queuedMessageCount, setQueuedMessageCount] = useState(0);
  const [cancellingTurnId, setCancellingTurnId] = useState<string | null>(null);
  const [pendingApprovalIds, setPendingApprovalIds] = useState<string[]>([]);
  const [decidingApprovalId, setDecidingApprovalId] = useState<string | null>(
    null,
  );
  const [approvalDecisionError, setApprovalDecisionError] = useState<
    string | null
  >(null);
  const [pendingUserInput, setPendingUserInput] = useState<UserInputRecord[]>(
    [],
  );
  const [submittingUserInputId, setSubmittingUserInputId] = useState<
    string | null
  >(null);
  const [userInputError, setUserInputError] = useState<string | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
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
  const [activeToolTabId, setActiveToolTabId] = useState<string | null>(null);
  const [toolStageOpen, setToolStageOpen] = useState(false);
  const [conversationCollapsed, setConversationCollapsed] = useState(false);
  const [contextRailOpen, setContextRailOpen] = useState(false);
  const [contextRailCollapsed, setContextRailCollapsed] = useState(false);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [taskSearchOpen, setTaskSearchOpen] = useState(false);
  const [keyboardShortcutsOpen, setKeyboardShortcutsOpen] = useState(false);
  const [aboutOpen, setAboutOpen] = useState(false);
  const [draftProjectId, setDraftProjectId] = useState<string | null>(null);
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
  const taskNotificationPreferencesRef = useRef(taskNotificationPreferences);
  const ingestedEventIdsRef = useRef(new Set<string>());
  const pendingEventRenderRef = useRef<AgentEvent[]>([]);
  const eventRenderFrameRef = useRef<number | null>(null);
  const pendingEventCommitTraceRef = useRef(
    new Map<
      string,
      {
        eventTrace: ConversationStreamEventTrace;
        receivedClockMs: number;
      }
    >(),
  );
  const activeThreadIdRef = useRef<string | null>(null);

  activeThreadIdRef.current = activeThreadId;

  const setThreadActivityStatus = useCallback(
    (threadId: string, status: ThreadActivityStatus | null) => {
      setThreadActivityStatuses((current) => {
        if (!status) {
          if (!(threadId in current)) return current;
          const next = { ...current };
          delete next[threadId];
          return next;
        }
        if (current[threadId] === status) return current;
        return { ...current, [threadId]: status };
      });
    },
    [],
  );

  const activeThread = useMemo(
    () => threads.find((thread) => thread.id === activeThreadId) ?? null,
    [threads, activeThreadId],
  );
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
  const conversationMessages = isConversationReady ? messages : [];
  const conversationEvents = isConversationReady ? events : [];
  const conversationGoalSnapshot = isConversationReady ? goalSnapshot : null;
  const conversationActiveTurnId = isConversationReady ? activeTurnId : null;
  const conversationPendingTurnFeedback =
    isConversationReady && pendingTurnFeedback?.threadId === activeThreadId
      ? pendingTurnFeedback
      : null;
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
  const pendingApprovalQueue = useMemo(
    () =>
      conversationEvents
        .filter(
          (event): event is AgentEvent & { payload: ApprovalRequest } =>
            event.payload.type === "approval_requested" &&
            pendingApprovalIds.includes(event.payload.approval_id),
        )
        .sort((a, b) => a.seq - b.seq),
    [conversationEvents, pendingApprovalIds],
  );
  const activeApproval = pendingApprovalQueue[0]?.payload ?? null;
  const activeUserInput = isConversationReady
    ? (pendingUserInput[0] ?? null)
    : null;
  const composerTaskPlan = useMemo(
    () => resolveComposerTaskPlan(conversationEvents, conversationGoalSnapshot),
    [conversationEvents, conversationGoalSnapshot],
  );

  useEffect(() => {
    setQueuedMessageCount(0);
    setTurnUndoDialog(null);
  }, [activeThreadId]);

  useEffect(
    () => () => {
      if (eventRenderFrameRef.current !== null) {
        window.cancelAnimationFrame(eventRenderFrameRef.current);
        eventRenderFrameRef.current = null;
      }
      pendingEventRenderRef.current = [];
      pendingEventCommitTraceRef.current.clear();
    },
    [activeThreadId],
  );

  useEffect(() => {
    if (!pendingTurnFeedback) return;
    const feedbackHasResolved = events.some((event) => {
      const feedbackEvent =
        event.payload.type === "turn_started" ||
        event.payload.type === "turn_finished" ||
        event.payload.type === "turn_suspended" ||
        event.payload.type === "turn_cancelled" ||
        event.payload.type === "turn_awaiting_input" ||
        event.payload.type === "error";
      if (!feedbackEvent) return false;
      return pendingTurnFeedback.turnId
        ? event.turnId === pendingTurnFeedback.turnId
        : event.createdAt >= pendingTurnFeedback.startedAt;
    });
    if (!feedbackHasResolved) return;
    setPendingTurnFeedback((current) =>
      current?.startedAt === pendingTurnFeedback.startedAt ? null : current,
    );
  }, [events, pendingTurnFeedback]);

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
      /answer the pending planning question before starting another turn/i.test(
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
        toolStageOpen,
        conversationCollapsed,
      ),
    [
      conversationCollapsed,
      toolStageOpen,
      workspaceLayoutPreferences,
      workspaceWidth,
    ],
  );
  const contextRailAutoVisible =
    !toolStageOpen &&
    workspaceWidth - (sidebarCollapsed ? 0 : workspaceLayout.left) >=
      contextRailInlineMinWidth;
  const contextRailVisible =
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
      .listSkills(currentWorkspaceRoot, activeThread?.id)
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
  }, [activeThread?.id, client, currentWorkspaceRoot, skillsRevision]);

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
    (summary: string, force = false) => {
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
        const body =
          summary.replace(/\s+/g, " ").trim().slice(0, 260) ||
          "OpenTopia 已完成当前任务。";
        void showSystemNotification({
          title: "任务已完成",
          body,
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
      if (event.threadId !== activeThreadIdRef.current) return;
      if (ingestedEventIdsRef.current.has(event.id)) return;
      ingestedEventIdsRef.current.add(event.id);
      if (ingestedEventIdsRef.current.size > 4096) {
        const oldestId = ingestedEventIdsRef.current.values().next().value;
        if (oldestId) ingestedEventIdsRef.current.delete(oldestId);
      }

      const eventTrace = conversationStreamEventTrace(event);
      if (eventTrace) {
        const traceTime = rendererTraceTime();
        pendingEventCommitTraceRef.current.set(event.id, {
          eventTrace,
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

      pendingEventRenderRef.current.push(event);
      if (eventRenderFrameRef.current === null) {
        eventRenderFrameRef.current = window.requestAnimationFrame(() => {
          eventRenderFrameRef.current = null;
          const pending = pendingEventRenderRef.current.filter(
            (queuedEvent) => queuedEvent.threadId === activeThreadIdRef.current,
          );
          pendingEventRenderRef.current = [];
          if (pending.length === 0) return;

          // Streaming events can arrive faster than the renderer can paint.
          setEvents((current) => {
            const knownIds = new Set(current.map((item) => item.id));
            const additions: AgentEvent[] = [];
            for (const queuedEvent of pending) {
              if (knownIds.has(queuedEvent.id)) continue;
              knownIds.add(queuedEvent.id);
              additions.push(queuedEvent);
            }
            return additions.length > 0
              ? [...current, ...additions].sort((a, b) => a.seq - b.seq)
              : current;
          });
        });
      }

      if (event.payload.type === "assistant_message") {
        const assistantMessage = event.payload.message;
        setMessages((current) => {
          if (current.some((message) => message.id === assistantMessage.id))
            return current;
          return [...current, assistantMessage];
        });
      }

      if (event.payload.type === "goal_updated") {
        setGoalSnapshot(event.payload.snapshot);
      }

      if (event.payload.type === "approval_requested") {
        const approvalId = event.payload.approval_id;
        setThreadActivityStatus(event.threadId, "approval");
        setApprovalDecisionError(null);
        setConversationCollapsed(false);
        setPendingApprovalIds((current) =>
          current.includes(approvalId) ? current : [...current, approvalId],
        );
      }

      if (event.payload.type === "browser_handoff_required") {
        setThreadActivityStatus(event.threadId, "user_action");
        setConversationCollapsed(false);
      }

      if (event.payload.type === "user_input_requested") {
        const request = event.payload.request;
        setUserInputError(null);
        setConversationCollapsed(false);
        setPendingUserInput((current) =>
          current.some(
            (record) => record.request.requestId === request.requestId,
          )
            ? current
            : [
                ...current,
                {
                  threadId: event.threadId,
                  request,
                  status: "pending",
                  response: null,
                  createdAt: event.createdAt,
                  answeredAt: null,
                },
              ],
        );
      }

      if (event.payload.type === "error") {
        setThreadActivityStatus(event.threadId, "failed");
        setActionError(
          `Agent 请求失败：${friendlyProviderError(event.payload.message)}`,
        );
      }

      if (event.payload.type === "turn_started" && event.turnId) {
        setThreadActivityStatus(event.threadId, "processing");
        setActiveTurnId(event.turnId);
        setCancellingTurnId(null);
        setQueuedMessageCount((current) => Math.max(0, current - 1));
      } else if (event.payload.type === "turn_finished") {
        setThreadActivityStatus(event.threadId, "succeeded");
        setActiveTurnId((current) =>
          !event.turnId || current === event.turnId ? null : current,
        );
        setCancellingTurnId((current) =>
          !event.turnId || current === event.turnId ? null : current,
        );
      } else if (event.payload.type === "turn_suspended") {
        setThreadActivityStatus(event.threadId, "approval");
        setActiveTurnId((current) =>
          !event.turnId || current === event.turnId ? null : current,
        );
        setCancellingTurnId((current) =>
          !event.turnId || current === event.turnId ? null : current,
        );
      } else if (event.payload.type === "browser_handoff_required") {
        setActiveTurnId((current) =>
          !event.turnId || current === event.turnId ? null : current,
        );
        setCancellingTurnId((current) =>
          !event.turnId || current === event.turnId ? null : current,
        );
      } else if (event.payload.type === "turn_cancelled") {
        setThreadActivityStatus(event.threadId, null);
        setActiveTurnId((current) =>
          !event.turnId || current === event.turnId ? null : current,
        );
        setCancellingTurnId((current) =>
          !event.turnId || current === event.turnId ? null : current,
        );
      } else if (
        event.payload.type === "turn_awaiting_input" ||
        event.payload.type === "error"
      ) {
        setActiveTurnId((current) =>
          !event.turnId || current === event.turnId ? null : current,
        );
        setCancellingTurnId((current) =>
          !event.turnId || current === event.turnId ? null : current,
        );
      }

      if (event.payload.type === "turn_finished") {
        deliverTaskCompletionNotification(event.payload.summary);
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

      if (event.payload.type === "subagent_updated") {
        const run = event.payload.run;
        setSubagentRuns((current) =>
          [run, ...current.filter((item) => item.id !== run.id)].sort(
            (left, right) => right.createdAt.localeCompare(left.createdAt),
          ),
        );
      }
    },
    [deliverTaskCompletionNotification, setThreadActivityStatus],
  );

  useLayoutEffect(() => {
    for (const event of events) {
      const pendingTrace = pendingEventCommitTraceRef.current.get(event.id);
      if (!pendingTrace) continue;
      pendingEventCommitTraceRef.current.delete(event.id);
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
  }, [events]);

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

        loadedThreads = await nextClient.listThreads(true);

        if (cancelled) return;
        setProjects(sortProjects(loadedProjects));
        setThreads(loadedThreads);
        void loadThreadActivityStatuses(nextClient, loadedThreads).then(
          (statuses) => {
            if (cancelled) return;
            const loadedThreadIds = new Set(
              loadedThreads.map((thread) => thread.id),
            );
            setThreadActivityStatuses((current) => ({
              ...statuses,
              ...Object.fromEntries(
                Object.entries(current).filter(([threadId]) =>
                  loadedThreadIds.has(threadId),
                ),
              ),
            }));
          },
        );
        setSettings(loadedSettings);
        setProviderHealth(loadedHealth);
        setMcpServers(loadedMcp);
        const projectIds = new Set(loadedProjects.map((project) => project.id));
        const firstVisibleThread = loadedThreads.find(
          (thread) =>
            !thread.archivedAt &&
            thread.experienceMode === experienceMode &&
            thread.projectId &&
            projectIds.has(thread.projectId),
        );
        const firstProject = sortProjects(loadedProjects)[0] ?? null;
        setActiveThreadId(
          (current) => current ?? firstVisibleThread?.id ?? null,
        );
        if (!firstVisibleThread) {
          setDraftProjectId((current) => current ?? firstProject?.id ?? null);
        }
        setSelectedWorkspaceRoot(
          (current) =>
            current ??
            firstVisibleThread?.workspaceRoot ??
            firstProject?.workspaceRoot ??
            null,
        );
        setServerStatus("online");
        setServerError(null);
        setServerProbing(false);
      } catch (error) {
        if (cancelled) return;
        // Stay in "checking" for the first probes: a warm dev launch answers
        // within a second, and flashing the waiting screen is just noise.
        setServerStatus(bootstrapAttempt >= 2 ? "offline" : "checking");
        setServerError(error instanceof Error ? error.message : String(error));
        setServerProbing(false);
      }
    });
    void bootstrapping.catch((error) => {
      if (cancelled) return;
      setServerStatus(bootstrapAttempt >= 2 ? "offline" : "checking");
      setServerError(error instanceof Error ? error.message : String(error));
      setServerProbing(false);
    });

    return () => {
      cancelled = true;
    };
  }, [bootstrapAttempt]);

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

  // In dev the backend is spawned through `cargo run`, so a cold or post-edit
  // build routinely outlasts the first health probe. Keep probing instead of
  // stranding the window on the offline screen until someone reloads it.
  useEffect(() => {
    if (serverStatus === "online" || serverProbing) return;
    const timer = setTimeout(
      () => setBootstrapAttempt((attempt) => attempt + 1),
      bootstrapAttempt < 4 ? 500 : 2000,
    );
    return () => clearTimeout(timer);
  }, [serverStatus, serverProbing, bootstrapAttempt]);

  // New tasks start on the newest stable model of the active connection. This is
  // why there is no "quality vs. speed" preference to configure: the latest
  // model is the default, and the composer is where you deviate from it.
  useEffect(() => {
    if (!settings) return;
    const active =
      settings.providers.find(
        (provider) => provider.id === settings.activeProviderId,
      ) ?? settings.providers[0];
    if (!active) return;
    setDraftModelSelection((current) => {
      if (current && current.connectionId === active.id) return current;
      const modelIds =
        active.syncedModels.length > 0 ? active.syncedModels : [active.model];
      const modelId = resolveDefaultModelId(
        modelIds,
        active.enabledFamilies,
        active.model,
      );
      return {
        connectionId: active.id,
        modelId,
        reasoningEffort: reconcileReasoningEffort(
          active.kind,
          modelId,
          active.reasoningEffort ?? null,
        ),
      };
    });
  }, [settings]);

  useEffect(() => {
    setPendingApprovalIds([]);
    setDecidingApprovalId(null);
    setApprovalDecisionError(null);
    setPendingUserInput([]);
    setSubmittingUserInputId(null);
    setUserInputError(null);
    setActiveTurnId(null);
    setCancellingTurnId(null);
    setGoalSnapshot(null);
    if (!client || !activeThreadId) {
      setMessages([]);
      setEvents([]);
      setConversationLoadState({
        threadId: null,
        status: "idle",
        error: null,
      });
      return;
    }
    const threadId = activeThreadId;
    let cancelled = false;
    let source: StreamHandle | null = null;
    setMessages([]);
    setEvents([]);
    setConversationLoadState({ threadId, status: "loading", error: null });

    const messagesRequest = client.listMessages(threadId);
    const eventsRequest = client.listEvents(threadId);

    void messagesRequest
      .then((loadedMessages) => {
        if (cancelled) return;
        setMessages(loadedMessages);
        setConversationLoadState({ threadId, status: "ready", error: null });
      })
      .catch((error) => {
        if (cancelled) return;
        const message = error instanceof Error ? error.message : String(error);
        setConversationLoadState({
          threadId,
          status: "error",
          error: message,
        });
        setServerError(message);
      });

    void eventsRequest
      .then((loadedEvents) => {
        if (cancelled) return;
        setEvents(loadedEvents);
        source = client.openEventStream(
          threadId,
          loadedEvents.at(-1)?.seq,
          ingestEvent,
        );
      })
      .catch((error) => {
        if (!cancelled)
          setServerError(
            error instanceof Error ? error.message : String(error),
          );
      });

    void Promise.all([
      client.getTurnStatus(threadId),
      client.listPendingApprovals(threadId),
      client.listPendingUserInput(threadId),
      client.listSubagents(threadId),
      client.getGoal(threadId),
    ])
      .then(
        ([
          turnStatus,
          pendingApprovals,
          pendingPlanningInput,
          loadedSubagents,
          loadedGoal,
        ]) => {
          if (cancelled) return;
          setActiveTurnId(
            turnStatus?.status === "running" ||
              turnStatus?.status === "cancelling"
              ? turnStatus.turnId
              : null,
          );
          setThreadActivityStatus(
            threadId,
            pendingApprovals.length > 0
              ? "approval"
              : resolveThreadActivityStatus(turnStatus),
          );
          setPendingApprovalIds(
            pendingApprovals.map((approval) => approval.approvalId),
          );
          setPendingUserInput(pendingPlanningInput);
          setSubagentRuns(loadedSubagents);
          setGoalSnapshot(loadedGoal);
        },
      )
      .catch((error) => {
        if (!cancelled)
          setServerError(
            error instanceof Error ? error.message : String(error),
          );
      });

    return () => {
      cancelled = true;
      source?.close();
    };
  }, [
    activeThreadId,
    client,
    conversationLoadAttempt,
    ingestEvent,
    setThreadActivityStatus,
  ]);

  useEffect(() => {
    if (!client || !activeThreadId) {
      setTerminalEvents([]);
      setTerminalSession(null);
      return;
    }
    let cancelled = false;
    let source: StreamHandle | null = null;
    setTerminalEvents([]);
    setTerminalSession(null);

    void (async () => {
      const history = await client.listTerminalHistory(activeThreadId);
      if (cancelled) return;
      setTerminalEvents(history);
      const since = history.at(-1)?.seq;
      source = client.openTerminalStream(
        activeThreadId,
        since,
        ingestTerminalEvent,
      );
      const session = await client.ensureTerminalSession(activeThreadId);
      if (!cancelled) setTerminalSession(session);
    })().catch((error) => {
      if (!cancelled)
        setWorkbenchError(
          error instanceof Error ? error.message : String(error),
        );
    });

    return () => {
      cancelled = true;
      source?.close();
    };
  }, [activeThreadId, client, ingestTerminalEvent]);

  const refreshWorkbench = useCallback(
    async (path?: string) => {
      if (!client || !activeThreadId) return;
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
        ] = await Promise.all([
          client.listWorkspaceTree(activeThreadId, path),
          client.getWorkspaceDiff(activeThreadId),
          client.getSandbox(activeThreadId),
          client.listThreadMcpServers(activeThreadId),
          client.listArtifacts(activeThreadId),
          client.getContextStatus(activeThreadId),
        ]);
        setWorkspaceTree(tree);
        setWorkspaceDiff(diff);
        setSandbox(sandboxStatus);
        setThreadMcpServers(threadMcp);
        setArtifacts(artifactList);
        setContextStatus(loadedContextStatus);
        setMcpServers(await client.listMcpServers());
      } catch (error) {
        setWorkbenchError(
          error instanceof Error ? error.message : String(error),
        );
      } finally {
        setIsRefreshingWorkbench(false);
      }
    },
    [activeThreadId, client],
  );

  useEffect(() => {
    if (!activeThreadId) {
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
  }, [activeThreadId, refreshWorkbench]);

  function selectThread(threadId: string) {
    const thread = threads.find((item) => item.id === threadId);
    setActiveThreadId(threadId);
    if (thread) setExperienceMode(thread.experienceMode);
    setDraftProjectId(null);
    setContextSources([]);
    setSelectedSkillIds([]);
    if (thread?.workspaceRoot) setSelectedWorkspaceRoot(thread.workspaceRoot);
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
        setActiveThreadId(thread.id);
        setExperienceMode(thread.experienceMode);
        setDraftProjectId(null);
        setContextSources([]);
        setSelectedSkillIds([]);
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
    setActiveThreadId(null);
    setMessages([]);
    setEvents([]);
    setComposer("");
    setNewTaskLaunchMode("local");
    setContextSources([]);
    setSelectedSkillIds([]);
    setActiveTurnId(null);
    setPendingApprovalIds([]);
    setToolTabs([]);
    setActiveToolTabId(null);
    setToolStageOpen(false);
    setConversationCollapsed(false);
    setSelectedWorkspaceRoot(workspaceRoot);
    setDraftProjectId(projectId);
  }

  function changeExperienceMode(nextMode: ExperienceMode) {
    if (nextMode === experienceMode) return;
    const project = activeProject ?? draftProject;
    setExperienceMode(nextMode);
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
    if (kind === "preview" || kind === "side-task") return;
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

  function toggleToolPanel(kind: Exclude<ToolTabKind, "preview">) {
    const tabId = `tool-${kind}`;
    if (toolStageOpen && activeToolTabId === tabId) {
      setToolStageOpen(false);
      setConversationCollapsed(false);
      return;
    }
    openToolTab(kind);
  }

  const openPreviewTab = useCallback(function openPreviewTab(
    threadId: string,
    target: PreviewTarget,
    title: string,
  ) {
    const targetKey =
      target.type === "workspace"
        ? `workspace:${target.path}`
        : target.type === "artifact"
          ? `artifact:${target.artifactId}`
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
    setToolTabs((current) => {
      const closingIndex = current.findIndex((tab) => tab.id === tabId);
      const next = current.filter((tab) => tab.id !== tabId);
      if (activeToolTabId === tabId) {
        const replacement =
          next[Math.min(Math.max(closingIndex, 0), next.length - 1)] ?? null;
        setActiveToolTabId(replacement?.id ?? null);
        if (!replacement) setConversationCollapsed(false);
      }
      return next;
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
        client.listSkills(currentWorkspaceRoot, activeThread?.id),
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
    permissionMode?: "chat" | "read_only" | "auto" | "approve" | "full_access";
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
    permissionMode: "auto" | "approve" | "full_access",
  ) {
    if (!settings || isSavingSettings || activeTurnId) return;
    if (
      permissionMode === "full_access" &&
      !window.confirm(
        "完全访问权限将允许访问互联网和此电脑上的任意文件。确认继续？",
      )
    ) {
      return;
    }
    void saveSettings({
      permissionMode,
      sandbox:
        permissionMode === "full_access"
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
    void saveSettings({
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

  async function addContextSources(files?: File[]): Promise<void> {
    setActionError(null);
    try {
      const result = files
        ? await getDroppedContextFiles(files)
        : await selectContextFiles({
            defaultPath: currentWorkspaceRoot ?? undefined,
          });
      if (result.canceled) return;
      setContextSources((current) => {
        const byPath = new Map(
          current.map((source) => [workspaceRootKey(source.path), source]),
        );
        for (const source of result.files) {
          byPath.set(workspaceRootKey(source.path), source);
        }
        return [...byPath.values()].slice(0, 20);
      });
    } catch (error) {
      setActionError(`添加来源失败：${errorMessage(error)}`);
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

  async function cancelSubagent(runId: string) {
    if (!client || !activeThread) return;
    setActionError(null);
    try {
      await client.cancelSubagent(activeThread.id, runId);
    } catch (error) {
      setActionError(`取消子智能体失败：${errorMessage(error)}`);
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

  async function prepareImageSubmission(
    threadId: string,
    prompt: string,
    imageAttachments: InlineImageAttachment[],
    modelSelection: ThreadModelSelection | null,
  ): Promise<{
    prompt: string;
    imageAttachments: InlineImageAttachment[];
  }> {
    if (!client || imageAttachments.length === 0) {
      return { prompt, imageAttachments };
    }

    const providerId =
      modelSelection?.connectionId ?? settings?.activeProviderId;
    const provider = settings?.providers.find((item) => item.id === providerId);
    if (
      provider &&
      modelSupportsVision(provider, modelSelection?.modelId ?? provider.model)
    ) {
      return { prompt, imageAttachments };
    }

    const threadServers = await client.listThreadMcpServers(threadId);
    const configuredServers = threadServers.filter(
      (view) => view.server.enabled,
    );
    const failures: string[] = [];

    for (const view of configuredServers) {
      let tools: McpToolDescriptor[];
      try {
        tools = await client.listMcpTools(view.server.serverId);
      } catch (error) {
        failures.push(`${view.server.name}: ${errorMessage(error)}`);
        continue;
      }
      const tool = tools.find(isImageUnderstandingMcpTool);
      if (!tool) continue;

      try {
        if (!view.enabled) {
          await client.setThreadMcpServer(threadId, view.server.serverId, true);
        }
        const result = await client.callMcpTool(
          view.server.serverId,
          tool.toolName,
          buildImageUnderstandingArguments(tool, prompt, imageAttachments),
          threadId,
        );
        if (result.isError) {
          failures.push(
            `${view.server.name}: ${result.output || "tool returned an error"}`,
          );
          continue;
        }
        const understanding = extractImageUnderstandingText(result);
        if (!understanding) {
          failures.push(`${view.server.name}: tool returned no text`);
          continue;
        }
        return {
          prompt: appendImageUnderstandingContext(prompt, understanding),
          imageAttachments: [],
        };
      } catch (error) {
        failures.push(`${view.server.name}: ${errorMessage(error)}`);
      }
    }

    const detail = failures.length > 0 ? ` (${failures.join("; ")})` : "";
    throw new Error(
      `The selected model does not support image input and no usable image understanding MCP is configured${detail}. Your message and images were kept in the composer.`,
    );
  }

  async function createThread(
    initialPrompt?: string,
    imageAttachments: InlineImageAttachment[] = [],
  ): Promise<boolean> {
    if (!client) return false;
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

    setIsSending(
      Boolean(initialPrompt?.trim()) ||
        contextSources.length > 0 ||
        imageAttachments.length > 0 ||
        selectedSkillIds.length > 0,
    );
    let pendingFeedbackStartedAt: string | null = null;
    setActionError(null);
    try {
      const prompt = initialPrompt?.trim() ?? "";
      let thread = await client.createThread({
        title: prompt ? threadTitleFromPrompt(prompt) : project.name,
        workspaceRoot: project.workspaceRoot,
        projectId: project.id,
        experienceMode,
      });
      if (draftModelSelection) {
        // Pin before the first turn runs, so the conversation starts on the
        // model picked in the draft composer rather than the connection default.
        try {
          thread = await client.setThreadModel(thread.id, draftModelSelection);
        } catch (error) {
          console.warn("OpenTopia could not pin the task model", error);
        }
      }
      setThreads((current) => [thread, ...current]);
      setActiveThreadId(thread.id);
      setSelectedWorkspaceRoot(thread.workspaceRoot);
      setDraftProjectId(null);
      setToolTabs([]);
      setActiveToolTabId(null);
      setToolStageOpen(false);
      if (threadTitleNeedsSummary(prompt)) {
        void client
          .generateThreadTitle(thread.id, prompt, thread.title)
          .then(({ thread: titledThread, updated }) => {
            if (!updated) return;
            setThreads((current) =>
              current.map((item) =>
                item.id === titledThread.id ? titledThread : item,
              ),
            );
          })
          .catch((error) => {
            console.warn("OpenTopia could not generate the task title", error);
          });
      }
      if (directCommand) {
        await runDirectToolCommand(thread.id, directCommand);
        setComposer("");
      } else if (
        initialPrompt?.trim() ||
        contextSources.length > 0 ||
        imageAttachments.length > 0 ||
        selectedSkillIds.length > 0
      ) {
        pendingFeedbackStartedAt = new Date().toISOString();
        setPendingTurnFeedback({
          threadId: thread.id,
          turnId: null,
          phase: "thinking",
          startedAt: pendingFeedbackStartedAt,
        });
        const prepared = await prepareImageSubmission(
          thread.id,
          initialPrompt?.trim() ?? "",
          imageAttachments,
          draftModelSelection,
        );
        const { message, turnId } = await client.sendMessage(
          thread.id,
          prepared.prompt,
          contextSources.map((source) => source.path),
          selectedSkillIds,
          collaborationMode,
          undefined,
          prepared.imageAttachments,
        );
        setMessages([message]);
        setThreadActivityStatus(thread.id, "processing");
        if (turnId) setActiveTurnId(turnId);
        setPendingTurnFeedback((current) =>
          current?.startedAt === pendingFeedbackStartedAt
            ? { ...current, turnId }
            : current,
        );
        setComposer("");
        setContextSources([]);
        setSelectedSkillIds([]);
      }
      return true;
    } catch (error) {
      if (pendingFeedbackStartedAt) {
        setPendingTurnFeedback((current) =>
          current?.startedAt === pendingFeedbackStartedAt ? null : current,
        );
      }
      setActionError(`创建任务失败：${errorMessage(error)}`);
      return false;
    } finally {
      setIsSending(false);
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
    setIsSending(true);
    let pendingFeedbackStartedAt: string | null = null;
    try {
      if (directCommand) {
        await runDirectToolCommand(activeThread.id, directCommand);
        setComposer("");
        return true;
      }
      pendingFeedbackStartedAt = new Date().toISOString();
      setPendingTurnFeedback({
        threadId: activeThread.id,
        turnId: null,
        phase: "thinking",
        startedAt: pendingFeedbackStartedAt,
      });
      const prepared = await prepareImageSubmission(
        activeThread.id,
        messageText,
        imageAttachments,
        activeThread.modelSelection,
      );
      const { message, turnId, queued } = await client.sendMessage(
        activeThread.id,
        prepared.prompt,
        contextSources.map((source) => source.path),
        selectedSkillIds,
        collaborationMode,
        reusableGoalId(collaborationMode, goalSnapshot),
        prepared.imageAttachments,
      );
      setMessages((current) => [...current, message]);
      setThreadActivityStatus(activeThread.id, "processing");
      if (turnId) setActiveTurnId(turnId);
      setPendingTurnFeedback((current) =>
        current?.startedAt === pendingFeedbackStartedAt
          ? {
              ...current,
              turnId,
              phase: queued ? "processing" : current.phase,
            }
          : current,
      );
      if (queued) setQueuedMessageCount((current) => current + 1);
      setComposer("");
      setContextSources([]);
      setSelectedSkillIds([]);
      try {
        const turnStatus = await client.getTurnStatus(activeThread.id);
        setActiveTurnId(
          turnStatus?.status === "running" ||
            turnStatus?.status === "cancelling"
            ? turnStatus.turnId
            : null,
        );
      } catch {
        // The persisted event stream will reconcile Turn state after a successful send.
      }
      return true;
    } catch (error) {
      if (pendingFeedbackStartedAt) {
        setPendingTurnFeedback((current) =>
          current?.startedAt === pendingFeedbackStartedAt ? null : current,
        );
      }
      setActionError(errorMessage(error));
      return false;
    } finally {
      setIsSending(false);
    }
  }

  async function cancelTurn() {
    if (
      !client ||
      !activeThread ||
      !activeTurnId ||
      cancellingTurnId === activeTurnId
    )
      return;
    const turnId = activeTurnId;
    setCancellingTurnId(turnId);
    setActionError(null);
    try {
      const result = await client.cancelTurn(activeThread.id, turnId);
      if (!result.cancelled) {
        setCancellingTurnId(null);
        setActionError(result.message);
      }
    } catch (error) {
      setCancellingTurnId(null);
      setActionError(`中断执行失败：${errorMessage(error)}`);
    }
  }

  async function runGoal() {
    if (!client || !activeThread || !goalSnapshot || activeTurnId) return;
    const goalId = goalSnapshot.goal.id;
    setGoalAction("run");
    setActionError(null);
    try {
      let snapshot = goalSnapshot;
      if (snapshot.goal.status !== "active") {
        snapshot = await client.updateGoalStatus(
          activeThread.id,
          goalId,
          "active",
        );
        setGoalSnapshot(snapshot);
      }
      setCollaborationMode("goal");
      const { message, turnId, queued } = await client.sendMessage(
        activeThread.id,
        "继续执行已确认的目标计划，直到完成或出现明确阻塞。",
        [],
        [],
        "goal",
        goalId,
      );
      setMessages((current) => [...current, message]);
      if (turnId) setActiveTurnId(turnId);
      if (queued) setQueuedMessageCount((current) => current + 1);
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
        const result = await client.cancelTurn(activeThread.id, activeTurnId);
        if (!result.cancelled) throw new Error(result.message);
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
    if (!client || !activeThread || decidingApprovalId) return;
    setDecidingApprovalId(approvalId);
    setApprovalDecisionError(null);
    try {
      const decision = await client.decideApproval(
        activeThread.id,
        approvalId,
        approved,
      );
      if (!decision.accepted) {
        throw new Error("服务端未接受该审批决定，请重试。");
      }
      setPendingApprovalIds((current) =>
        current.filter((id) => id !== approvalId),
      );
    } catch (error) {
      setApprovalDecisionError(
        `审批决定提交失败：${error instanceof Error ? error.message : String(error)}`,
      );
    } finally {
      setDecidingApprovalId(null);
    }
  }

  async function submitUserInput(
    requestId: string,
    response: UserInputResponse,
  ) {
    if (!client || !activeThread || submittingUserInputId) return;
    setSubmittingUserInputId(requestId);
    setUserInputError(null);
    try {
      const result = await client.respondToUserInput(
        activeThread.id,
        requestId,
        response,
      );
      if (!result.accepted || !result.resumed) {
        throw new Error("服务端未恢复规划，请重试。");
      }
      setPendingUserInput((current) =>
        current.filter((record) => record.request.requestId !== requestId),
      );
    } catch (error) {
      setUserInputError(`无法提交选择：${errorMessage(error)}`);
    } finally {
      setSubmittingUserInputId(null);
    }
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
      setEvents((current) => [
        ...current,
        {
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
        },
      ]);
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

      setEvents((current) => [
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
      setEvents((current) => [
        ...current,
        {
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
        },
      ]);
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
      const login = await client.startCodexLogin(true);
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
      preferenceKey: isLeft ? "left" : "toolRight",
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
      isLeft ? "left" : "toolRight",
      clampPanelSize(next, min, max),
    );
  }

  function resetWorkspacePanelSize(side: WorkspaceResizeSide) {
    const key: keyof WorkspaceLayoutPreferences =
      side === "left" ? "left" : "toolRight";
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
                provider.id === providerId
                  ? {
                      ...provider,
                      model: result.defaultModel,
                      syncedModels: result.models,
                      modelContextWindows: result.modelContextWindows,
                      modelCapabilities: result.modelCapabilities,
                      modelsSyncedAt: result.syncedAt,
                    }
                  : provider,
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

    if (!activeThreadId) {
      setDraftModelSelection(selection);
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

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing) return;
      if (!(event.ctrlKey || event.metaKey)) return;

      const key = event.key.toLocaleLowerCase();
      if (taskSearchOpen) return;
      if (key === ",") {
        event.preventDefault();
        if (!settingsOpen) setSettingsOpen(true);
        return;
      }
      if (
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
        toggleToolPanel("browser");
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
          onNewTask={beginNewThread}
          onOpenWorkspace={() => void chooseWorkspace()}
          onToggleTool={toggleToolPanel}
          onOpenSettings={() => setSettingsOpen(true)}
          onOpenLogs={() => setLogViewerOpen(true)}
          onShowKeyboardShortcuts={() => setKeyboardShortcutsOpen(true)}
          onShowAbout={() => setAboutOpen(true)}
          menuSuppressed={
            settingsOpen || keyboardShortcutsOpen || aboutOpen || taskSearchOpen
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
        {!settingsOpen ? (
          <main
            ref={workspaceRef}
            className={`workspace ${toolStageOpen ? "with-tool-stage" : ""} ${conversationCollapsed ? "tool-only" : ""} ${sidebarCollapsed ? "sidebar-collapsed" : ""} ${workspaceResizeSide ? "is-resizing" : ""}`}
            style={workspaceStyle}
          >
            <Sidebar
              client={client}
              projects={projects}
              threads={threads}
              threadActivityStatuses={threadActivityStatuses}
              activeThreadId={activeThreadId}
              activeProjectId={activeThread?.projectId ?? draftProjectId}
              activeWorkspaceRemoteUrl={workspaceDiff?.remoteUrl ?? null}
              workspaceError={workspaceError}
              isPickingWorkspace={isPickingWorkspace}
              experienceMode={experienceMode}
              onExperienceModeChange={changeExperienceMode}
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
              onRestoreThread={(thread) => void restoreThread(thread)}
              onOpenThreadWorkspace={(workspaceRoot) =>
                void openWorkspaceRoot(workspaceRoot)
              }
              onOpenExtensions={() => openToolTab("extensions")}
              onOpenTaskSearch={() => setTaskSearchOpen(true)}
              onSettings={() => setSettingsOpen(true)}
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
              }`}
              id="workspace-center-pane"
            >
              <ThreadHeader
                thread={activeThread}
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
              {serverStatus === "offline" ? (
                <OfflineState
                  backendUrl={platform?.backendUrl}
                  error={serverError}
                  attempt={bootstrapAttempt}
                  isProbing={serverProbing}
                  onRetry={() => setBootstrapAttempt((attempt) => attempt + 1)}
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
                      onRetry={() =>
                        setConversationLoadAttempt((attempt) => attempt + 1)
                      }
                    />
                  ) : isConversationLoading ? (
                    <ConversationLoadingState />
                  ) : (
                    <MessageList
                      key={activeThread.id}
                      messages={conversationMessages}
                      events={conversationEvents}
                      activeTurnId={conversationActiveTurnId}
                      pendingTurnFeedback={conversationPendingTurnFeedback}
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
                      onOpenMarkdownLink={openMarkdownLink}
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
                    />
                  ) : (
                    <Composer
                      value={composer}
                      taskPlan={composerTaskPlan}
                      isSending={isSending}
                      isRunning={Boolean(conversationActiveTurnId)}
                      isCancelling={
                        Boolean(conversationActiveTurnId) &&
                        cancellingTurnId === conversationActiveTurnId
                      }
                      queuedMessageCount={
                        isConversationReady ? queuedMessageCount : 0
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
                      onOpenSettings={() => setSettingsOpen(true)}
                      onAddContextSources={addContextSources}
                      onRemoveContextSource={removeContextSource}
                      onToggleSkill={toggleSkill}
                    />
                  )}
                </>
              ) : (
                <NewTaskState
                  value={composer}
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
                  onOpenSettings={() => setSettingsOpen(true)}
                  onAddContextSources={addContextSources}
                  onRemoveContextSource={removeContextSource}
                  onToggleSkill={toggleSkill}
                  onSubmit={createThread}
                />
              )}
            </section>
            {toolStageOpen && !settingsOpen ? (
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
              threads={threads}
              toolTabs={toolTabs}
              activeToolTab={activeToolTab}
              toolStageOpen={toolStageOpen}
              conversationCollapsed={conversationCollapsed}
              contextRailOpen={contextRailVisible}
              contextRailAutoVisible={contextRailAutoVisible}
              thread={activeThread}
              settings={settings}
              projects={projects}
              skills={skills}
              collaborationMode={collaborationMode}
              workspaceRoot={currentWorkspaceRoot}
              messages={conversationMessages}
              events={conversationEvents.filter(
                (event) =>
                  event.payload.type !== "approval_requested" ||
                  pendingApprovalIds.includes(event.payload.approval_id),
              )}
              subagentRuns={isConversationReady ? subagentRuns : []}
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
              onOpenToolTab={openToolTab}
              onOpenSideTask={() => void openSideTask()}
              onThreadUpdated={(updatedThread) =>
                setThreads((current) =>
                  current.map((thread) =>
                    thread.id === updatedThread.id ? updatedThread : thread,
                  ),
                )
              }
              onSetThreadActivity={setThreadActivityStatus}
              onChangePermissionMode={changeExecutionPreset}
              onChangeSandboxMode={changeSandboxMode}
              onOpenSettings={() => setSettingsOpen(true)}
              onActivateToolTab={setActiveToolTabId}
              onCloseToolTab={closeToolTab}
              onToggleConversation={() =>
                setConversationCollapsed((current) => !current)
              }
              onHideToolStage={() => {
                setToolStageOpen(false);
                setConversationCollapsed(false);
              }}
              onAddContextSources={() => void addContextSources()}
              onCancelSubagent={(runId) => void cancelSubagent(runId)}
            />
          </main>
        ) : null}
        {settingsOpen && (
          <RedesignedSettingsPanel
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
                "测试成功：OpenTopia 可以在任务完成时提醒你。",
                true,
              )
            }
            onOpenLogs={() => {
              setSettingsOpen(false);
              setLogViewerOpen(true);
            }}
            onClose={() => setSettingsOpen(false)}
          />
        )}
        {taskSearchOpen ? (
          <TaskSearchDialog
            activeThreadId={activeThreadId}
            activityStatuses={threadActivityStatuses}
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

function TurnUndoDialog({
  state,
  onConfirm,
  onClose,
}: {
  state: TurnUndoDialogState;
  onConfirm(): void;
  onClose(): void;
}) {
  const { preview } = state;
  const files = preview?.changeSet.files ?? [];

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !state.applying) onClose();
    };
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [onClose, state.applying]);

  return (
    <div
      className="modal-backdrop project-modal-backdrop"
      role="presentation"
      onClick={() => {
        if (!state.applying) onClose();
      }}
    >
      <section
        className="turn-undo-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="turn-undo-dialog-title"
        aria-describedby="turn-undo-dialog-description"
        onClick={(event) => event.stopPropagation()}
      >
        <header>
          <div>
            <h2 id="turn-undo-dialog-title">撤销本轮修改</h2>
            <p id="turn-undo-dialog-description">
              使用当前工作区与该轮修改前后的快照进行三方合并。
            </p>
          </div>
          <button
            className="icon-button small"
            type="button"
            autoFocus
            aria-label="关闭撤销对话框"
            disabled={state.applying}
            onClick={onClose}
          >
            <X size={14} />
          </button>
        </header>

        {state.loading ? (
          <div className="turn-undo-loading" role="status">
            <Loader2 className="spin" size={16} />
            <span>正在检查当前文件与历史快照…</span>
          </div>
        ) : state.error ? (
          <div className="turn-undo-alert" role="alert">
            <AlertCircle size={16} />
            <span>{state.error}</span>
          </div>
        ) : preview ? (
          <>
            <div className="turn-undo-overview">
              <strong>{preview.changeSet.files.length} 个文件</strong>
              <span className="file-change-additions">
                +{preview.changeSet.additions}
              </span>
              <span className="file-change-deletions">
                -{preview.changeSet.deletions}
              </span>
            </div>

            {preview.conflicts.length > 0 ? (
              <div className="turn-undo-conflicts" role="alert">
                <strong>无法自动撤销</strong>
                <p>以下内容与该轮之后的修改发生冲突，工作区尚未更改。</p>
                <ul>
                  {preview.conflicts.map((conflict, index) => (
                    <li key={`${conflict.path ?? conflict.kind}-${index}`}>
                      <span>{conflict.path ?? "工作区"}</span>
                      <small>{conflict.reason}</small>
                    </li>
                  ))}
                </ul>
              </div>
            ) : (
              <div className="turn-undo-file-list" aria-label="将撤销的文件">
                {files.map((file, index) => {
                  const path = file.newPath ?? file.oldPath ?? "未知文件";
                  return (
                    <div key={`${file.kind}-${path}-${index}`}>
                      <span className="turn-undo-file-kind">
                        {turnFileChangeLabel(file.kind)}
                      </span>
                      <span title={path}>{path}</span>
                      <small>
                        <span className="file-change-additions">
                          +{file.additions ?? 0}
                        </span>{" "}
                        <span className="file-change-deletions">
                          -{file.deletions ?? 0}
                        </span>
                      </small>
                    </div>
                  );
                })}
              </div>
            )}
          </>
        ) : null}

        <footer>
          <button
            className="secondary-button"
            type="button"
            disabled={state.applying}
            onClick={onClose}
          >
            取消
          </button>
          {preview?.canUndo && (
            <button
              className="turn-undo-confirm"
              type="button"
              disabled={state.applying}
              onClick={onConfirm}
            >
              {state.applying ? (
                <Loader2 className="spin" size={14} />
              ) : (
                <RotateCcw size={14} />
              )}
              {state.applying ? "正在撤销" : "确认撤销"}
            </button>
          )}
        </footer>
      </section>
    </div>
  );
}

function turnFileChangeLabel(kind: string) {
  if (kind === "added") return "新增";
  if (kind === "deleted") return "删除";
  if (kind === "renamed") return "重命名";
  return "修改";
}

function RenameDialog({
  target,
  onSubmit,
  onClose,
}: {
  target: RenameTarget;
  onSubmit(name: string): Promise<boolean>;
  onClose(): void;
}) {
  const [name, setName] = useState(target.name);
  const [isSaving, setIsSaving] = useState(false);
  const label = target.kind === "project" ? "项目" : "任务";

  return (
    <div
      className="modal-backdrop project-modal-backdrop"
      role="presentation"
      onClick={onClose}
    >
      <form
        className="project-name-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="rename-dialog-title"
        onClick={(event) => event.stopPropagation()}
        onSubmit={(event) => {
          event.preventDefault();
          if (!name.trim() || isSaving) return;
          setIsSaving(true);
          void onSubmit(name).finally(() => setIsSaving(false));
        }}
      >
        <header>
          <div>
            <h2 id="rename-dialog-title">重命名{label}</h2>
            <p>名称将在所有项目视图中同步更新。</p>
          </div>
          <button
            className="icon-button small"
            type="button"
            aria-label="关闭重命名弹窗"
            onClick={onClose}
          >
            <X size={14} />
          </button>
        </header>
        <input
          autoFocus
          aria-label={`${label}名称`}
          value={name}
          onChange={(event) => setName(event.target.value)}
          onFocus={(event) => event.currentTarget.select()}
        />
        <footer>
          <button className="secondary-button" type="button" onClick={onClose}>
            取消
          </button>
          <button
            className="primary-button"
            type="submit"
            disabled={!name.trim() || isSaving}
          >
            {isSaving ? "保存中..." : "保存"}
          </button>
        </footer>
      </form>
    </div>
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

type TopBarMenu = "file" | "edit" | "view" | "help";
type NativeEditCommand =
  "undo" | "redo" | "cut" | "copy" | "paste" | "delete" | "selectAll";

function isEditableElement(value: EventTarget | null): value is HTMLElement {
  if (value instanceof HTMLTextAreaElement)
    return !value.disabled && !value.readOnly;
  if (value instanceof HTMLInputElement)
    return !value.disabled && !value.readOnly;
  return value instanceof HTMLElement && value.isContentEditable;
}

function TopBar({
  sidebarCollapsed,
  onToggleSidebar,
  onNewTask,
  onOpenWorkspace,
  onToggleTool,
  onOpenSettings,
  onOpenLogs,
  onShowKeyboardShortcuts,
  onShowAbout,
  menuSuppressed,
}: {
  sidebarCollapsed: boolean;
  onToggleSidebar(): void;
  onNewTask(): void;
  onOpenWorkspace(): void;
  onToggleTool(kind: Exclude<ToolTabKind, "preview">): void;
  onOpenSettings(): void;
  onOpenLogs(): void;
  onShowKeyboardShortcuts(): void;
  onShowAbout(): void;
  menuSuppressed: boolean;
}) {
  const [openMenu, setOpenMenu] = useState<TopBarMenu | null>(null);
  const [hasEditableTarget, setHasEditableTarget] = useState(false);
  const editableTargetRef = useRef<HTMLElement | null>(null);
  const menuRef = useDismissiblePopover(Boolean(openMenu), () =>
    setOpenMenu(null),
  );

  useEffect(() => {
    const rememberEditableTarget = (event: FocusEvent) => {
      if (!isEditableElement(event.target)) return;
      editableTargetRef.current = event.target;
      setHasEditableTarget(true);
    };
    document.addEventListener("focusin", rememberEditableTarget);
    return () =>
      document.removeEventListener("focusin", rememberEditableTarget);
  }, []);

  useEffect(() => {
    if (menuSuppressed) setOpenMenu(null);
  }, [menuSuppressed]);

  useEffect(() => {
    setOpenMenu(null);
  }, [sidebarCollapsed]);

  const toggleMenu = (menu: TopBarMenu) => {
    setOpenMenu((current) => (current === menu ? null : menu));
  };
  const closeMenu = () => setOpenMenu(null);
  const runAction = (action: () => void) => {
    action();
    closeMenu();
  };
  const runEditCommand = (command: NativeEditCommand) => {
    const target = editableTargetRef.current;
    if (!target || !target.isConnected || !isEditableElement(target)) {
      setHasEditableTarget(false);
      closeMenu();
      return;
    }
    target.focus({ preventScroll: true });
    if (command === "selectAll" && target instanceof HTMLInputElement) {
      target.select();
    } else if (
      command === "selectAll" &&
      target instanceof HTMLTextAreaElement
    ) {
      target.select();
    } else {
      document.execCommand(command);
    }
    closeMenu();
  };
  const preserveEditableFocus = (event: ReactPointerEvent<HTMLButtonElement>) =>
    event.preventDefault();

  const menuButton = (menu: TopBarMenu, label: string) => (
    <button
      className={`window-menu-item ${openMenu === menu ? "active" : ""}`}
      type="button"
      aria-haspopup="menu"
      aria-expanded={openMenu === menu}
      onClick={() => toggleMenu(menu)}
    >
      {label}
    </button>
  );

  return (
    <header className="topbar">
      <div className="window-menu" ref={menuRef}>
        <button
          className="window-app-button sidebar-toggle-button"
          type="button"
          aria-label={sidebarCollapsed ? "展开侧栏" : "折叠侧栏"}
          aria-pressed={sidebarCollapsed}
          title={sidebarCollapsed ? "展开侧栏 (Ctrl+B)" : "折叠侧栏 (Ctrl+B)"}
          onClick={() => runAction(onToggleSidebar)}
        >
          {sidebarCollapsed ? (
            <PanelLeftOpen size={15} aria-hidden="true" />
          ) : (
            <PanelLeftClose size={15} aria-hidden="true" />
          )}
          {sidebarCollapsed ? <span className="sidebar-toggle-dot" /> : null}
        </button>
        <button className="window-nav-button" disabled title="后退不可用">
          <ArrowLeft size={14} />
        </button>
        <button className="window-nav-button" disabled title="前进不可用">
          <ArrowRight size={14} />
        </button>

        <div className="window-menu-entry">
          {menuButton("file", "文件")}
          {openMenu === "file" ? (
            <div className="window-menu-popover" role="menu" aria-label="文件">
              <button
                type="button"
                role="menuitem"
                onClick={() => runAction(onNewTask)}
              >
                <span>新建任务</span>
                <kbd>Ctrl+N</kbd>
              </button>
              <button
                type="button"
                role="menuitem"
                disabled
                title="当前版本不支持多个应用窗口"
              >
                <span>新建窗口</span>
                <kbd>Ctrl+Shift+N</kbd>
              </button>
              <div className="window-menu-divider" role="separator" />
              <button
                type="button"
                role="menuitem"
                onClick={() => runAction(onOpenWorkspace)}
              >
                <span>打开工作区...</span>
                <kbd>Ctrl+O</kbd>
              </button>
              <div className="window-menu-divider" role="separator" />
              <button
                type="button"
                role="menuitem"
                disabled
                title="当前版本没有关闭任务的独立操作"
              >
                <span>关闭任务</span>
                <kbd>Ctrl+W</kbd>
              </button>
            </div>
          ) : null}
        </div>

        <div className="window-menu-entry">
          {menuButton("edit", "编辑")}
          {openMenu === "edit" ? (
            <div className="window-menu-popover" role="menu" aria-label="编辑">
              {(
                [
                  ["undo", "撤销", "Ctrl+Z"],
                  ["redo", "重做", "Ctrl+Y"],
                ] as const
              ).map(([command, label, shortcut]) => (
                <button
                  key={command}
                  type="button"
                  role="menuitem"
                  disabled={!hasEditableTarget}
                  onPointerDown={preserveEditableFocus}
                  onClick={() => runEditCommand(command)}
                >
                  <span>{label}</span>
                  <kbd>{shortcut}</kbd>
                </button>
              ))}
              <div className="window-menu-divider" role="separator" />
              {(
                [
                  ["cut", "剪切", "Ctrl+X"],
                  ["copy", "复制", "Ctrl+C"],
                  ["paste", "粘贴", "Ctrl+V"],
                  ["delete", "删除", ""],
                ] as const
              ).map(([command, label, shortcut]) => (
                <button
                  key={command}
                  type="button"
                  role="menuitem"
                  disabled={!hasEditableTarget}
                  onPointerDown={preserveEditableFocus}
                  onClick={() => runEditCommand(command)}
                >
                  <span>{label}</span>
                  <kbd>{shortcut}</kbd>
                </button>
              ))}
              <div className="window-menu-divider" role="separator" />
              <button
                type="button"
                role="menuitem"
                disabled={!hasEditableTarget}
                onPointerDown={preserveEditableFocus}
                onClick={() => runEditCommand("selectAll")}
              >
                <span>全选</span>
                <kbd>Ctrl+A</kbd>
              </button>
              <div className="window-menu-divider" role="separator" />
              <button
                type="button"
                role="menuitem"
                onClick={() => runAction(onOpenSettings)}
              >
                <span>设置...</span>
                <kbd>Ctrl+,</kbd>
              </button>
            </div>
          ) : null}
        </div>

        <div className="window-menu-entry">
          {menuButton("view", "视图")}
          {openMenu === "view" ? (
            <div
              className="window-menu-popover window-menu-popover-wide"
              role="menu"
              aria-label="视图"
            >
              <button
                type="button"
                role="menuitem"
                onClick={() => runAction(onToggleSidebar)}
              >
                <span>{sidebarCollapsed ? "展开侧栏" : "折叠侧栏"}</span>
                <kbd>Ctrl+B</kbd>
              </button>
              <button
                type="button"
                role="menuitem"
                disabled
                title="当前布局没有底部面板"
              >
                <span>切换底部面板</span>
                <kbd>Ctrl+J</kbd>
              </button>
              <button
                type="button"
                role="menuitem"
                disabled
                title="当前版本没有置顶摘要面板"
              >
                <span>切换置顶摘要</span>
                <kbd />
              </button>
              <button
                type="button"
                role="menuitem"
                onClick={() => runAction(() => onToggleTool("terminal"))}
              >
                <span>打开终端</span>
                <kbd>Ctrl+`</kbd>
              </button>
              <button
                type="button"
                role="menuitem"
                onClick={() => runAction(() => onToggleTool("files"))}
              >
                <span>切换文件树</span>
                <kbd>Ctrl+Shift+E</kbd>
              </button>
              <button
                type="button"
                role="menuitem"
                onClick={() => runAction(() => onToggleTool("diff"))}
              >
                <span>切换审查面板</span>
                <kbd>Ctrl+Alt+B</kbd>
              </button>
              <div className="window-menu-divider" role="separator" />
              <button
                type="button"
                role="menuitem"
                onClick={() => runAction(() => onToggleTool("browser"))}
              >
                <span>浏览器</span>
                <kbd />
              </button>
              <button
                type="button"
                role="menuitem"
                disabled
                title="当前版本没有全局查找面板"
              >
                <span>查找</span>
                <kbd>Ctrl+F</kbd>
              </button>
              <div className="window-menu-divider" role="separator" />
              <button
                type="button"
                role="menuitem"
                disabled
                title="当前版本不支持缩放界面"
              >
                <span>放大</span>
                <kbd>Ctrl+Shift+=</kbd>
              </button>
              <button
                type="button"
                role="menuitem"
                disabled
                title="当前版本不支持缩放界面"
              >
                <span>缩小</span>
                <kbd>Ctrl+-</kbd>
              </button>
              <button
                type="button"
                role="menuitem"
                disabled
                title="当前版本不支持缩放界面"
              >
                <span>实际大小</span>
                <kbd>Ctrl+0</kbd>
              </button>
              <div className="window-menu-divider" role="separator" />
              <button
                type="button"
                role="menuitem"
                disabled
                title="当前版本不支持全屏切换"
              >
                <span>切换全屏</span>
                <kbd>F11</kbd>
              </button>
            </div>
          ) : null}
        </div>

        <div className="window-menu-entry">
          {menuButton("help", "帮助")}
          {openMenu === "help" ? (
            <div className="window-menu-popover" role="menu" aria-label="帮助">
              <button
                type="button"
                role="menuitem"
                disabled
                title="当前版本没有在线文档入口"
              >
                <span>文档</span>
                <kbd />
              </button>
              <button
                type="button"
                role="menuitem"
                onClick={() => runAction(onShowKeyboardShortcuts)}
              >
                <span>键盘快捷键</span>
                <kbd>Ctrl+/</kbd>
              </button>
              <button
                type="button"
                role="menuitem"
                disabled
                title="当前版本没有更新公告"
              >
                <span>更新内容</span>
                <kbd />
              </button>
              <div className="window-menu-divider" role="separator" />
              <button
                type="button"
                role="menuitem"
                onClick={() => runAction(onOpenLogs)}
              >
                <span>故障排查（日志）</span>
                <kbd />
              </button>
              <button
                type="button"
                role="menuitem"
                disabled
                title="当前版本没有系统状态页"
              >
                <span>系统状态</span>
                <kbd />
              </button>
              <button
                type="button"
                role="menuitem"
                disabled
                title="当前版本没有反馈入口"
              >
                <span>发送反馈</span>
                <kbd />
              </button>
              <div className="window-menu-divider" role="separator" />
              <button
                type="button"
                role="menuitem"
                onClick={() => runAction(onShowAbout)}
              >
                <span>关于 OpenTopia</span>
                <kbd />
              </button>
            </div>
          ) : null}
        </div>
      </div>
    </header>
  );
}

function KeyboardShortcutsDialog({ onClose }: { onClose(): void }) {
  useEffect(() => {
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      onClose();
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [onClose]);

  const shortcuts = [
    ["新建任务", "Ctrl+N"],
    ["打开工作区", "Ctrl+O"],
    ["搜索任务", "Ctrl+K"],
    ["切换侧栏", "Ctrl+B"],
    ["设置", "Ctrl+,"],
    ["打开终端", "Ctrl+`"],
    ["打开浏览器", "Ctrl+T"],
    ["打开文件", "Ctrl+P"],
    ["侧边任务", "Ctrl+Alt+S"],
    ["切换文件树", "Ctrl+Shift+E"],
  ];

  return (
    <div
      className="modal-backdrop chrome-dialog-backdrop"
      role="presentation"
      onMouseDown={onClose}
    >
      <section
        className="chrome-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="keyboard-shortcuts-title"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header>
          <h2 id="keyboard-shortcuts-title">键盘快捷键</h2>
          <button
            className="icon-button"
            type="button"
            aria-label="关闭键盘快捷键"
            title="关闭"
            onClick={onClose}
          >
            <X size={17} />
          </button>
        </header>
        <dl className="chrome-shortcuts-list">
          {shortcuts.map(([label, shortcut]) => (
            <div key={shortcut}>
              <dt>{label}</dt>
              <dd>
                <kbd>{shortcut}</kbd>
              </dd>
            </div>
          ))}
        </dl>
      </section>
    </div>
  );
}

function AboutDialog({ onClose }: { onClose(): void }) {
  useEffect(() => {
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      onClose();
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [onClose]);

  return (
    <div
      className="modal-backdrop chrome-dialog-backdrop"
      role="presentation"
      onMouseDown={onClose}
    >
      <section
        className="chrome-dialog chrome-about-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="about-opentopia-title"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header>
          <h2 id="about-opentopia-title">OpenTopia</h2>
          <button
            className="icon-button"
            type="button"
            aria-label="关闭关于 OpenTopia"
            title="关闭"
            onClick={onClose}
          >
            <X size={17} />
          </button>
        </header>
        <p>本地优先的 AI 编码与工作代理。</p>
      </section>
    </div>
  );
}

function SettingsPanel({
  platform,
  settings,
  providerHealth,
  providerTest,
  secretSources,
  isSaving,
  isSavingSecret,
  onSave,
  onTestProvider,
  onStoreProviderApiKey,
  onDeleteProviderApiKey,
  onOpenLogs,
  onClose,
}: {
  platform: PlatformInfo | null;
  settings: AppSettings | null;
  providerHealth: ProviderHealth[];
  providerTest: {
    providerId: string;
    status: "testing" | "complete";
    result?: ProviderHealthCheckResult;
  } | null;
  secretSources: SecretSources | null;
  isSaving: boolean;
  isSavingSecret: boolean;
  onSave(input: {
    providers?: ProviderSettings[];
    activeProviderId?: string;
    providerKind?: ProviderKind;
    baseUrl?: string;
    model?: string;
    apiKeySource?: string;
    permissionMode?: "chat" | "read_only" | "auto" | "approve" | "full_access";
    sandbox?: AppSettings["sandbox"];
  }): void;
  onTestProvider(providerId: string, providers: ProviderSettings[]): void;
  onStoreProviderApiKey(
    providerId: string,
    value: string,
  ): Promise<ProviderSecretOutcome>;
  onDeleteProviderApiKey(providerId: string): Promise<ProviderSecretOutcome>;
  onOpenLogs(): void;
  onClose(): void;
}) {
  const [providers, setProviders] = useState<ProviderSettings[]>(
    settings?.providers ?? [],
  );
  const [activeProviderId, setActiveProviderId] = useState(
    settings?.activeProviderId ?? providers[0]?.id ?? "default",
  );
  const [editingProviderId, setEditingProviderId] = useState<string | null>(
    null,
  );
  const [permissionMode, setPermissionMode] = useState<
    "chat" | "read_only" | "auto" | "approve" | "full_access"
  >(settings?.permissionMode ?? "auto");
  const [sandboxSettings, setSandboxSettings] = useState<
    AppSettings["sandbox"]
  >(
    settings?.sandbox ?? {
      sandboxMode: "workspace-write",
      enforcement: "enforce",
      network: "allow",
      writableRoots: [],
      readPaths: [],
    },
  );
  const [providerApiKey, setProviderApiKey] = useState("");

  const editingProvider =
    providers.find((p) => p.id === editingProviderId) ?? providers[0] ?? null;
  useEffect(() => {
    if (settings) {
      setProviders(settings.providers);
      setActiveProviderId(settings.activeProviderId);
      setPermissionMode(settings.permissionMode);
      setSandboxSettings(settings.sandbox);
    }
  }, [settings]);

  useEffect(() => {
    setProviderApiKey("");
  }, [editingProviderId]);

  function updateProvider<K extends keyof ProviderSettings>(
    id: string,
    field: K,
    value: ProviderSettings[K],
  ) {
    setProviders((current) =>
      current.map((p) => (p.id === id ? { ...p, [field]: value } : p)),
    );
  }

  function addProvider() {
    const newId = `provider-${Date.now()}`;
    setProviders((current) => [
      ...current,
      {
        id: newId,
        name: "Custom provider",
        kind: "openai_compatible",
        baseUrl: "https://api.openai.com/v1",
        model: "gpt-4.1-mini",
        enabledFamilies: [],
        syncedModels: [],
        modelsSyncedAt: null,
        temperature: null,
        maxOutputTokens: null,
        contextWindowTokens: null,
        reasoningEffort: null,
        storeResponses: false,
        parallelToolCalls: false,
        promptCacheKey: null,
        promptCachePolicy: null,
        responsesCompactionThresholdTokens: null,
        rolloutBudget: null,
        supportsVision: true,
        apiKeySource: "OPENTOPIA_API_KEY",
        apiKeyConfigured: false,
        healthStatus: null,
      },
    ]);
    setEditingProviderId(newId);
  }

  function removeProvider(id: string) {
    setProviders((current) => {
      const next = current.filter((p) => p.id !== id);
      if (activeProviderId === id && next.length > 0) {
        setActiveProviderId(next[0].id);
      }
      if (editingProviderId === id) {
        setEditingProviderId(next[0]?.id ?? null);
      }
      return next;
    });
  }

  return (
    <div className="modal-backdrop" role="presentation" onClick={onClose}>
      <section
        className="settings-panel wide"
        role="dialog"
        aria-modal="true"
        onClick={(event) => event.stopPropagation()}
      >
        <header>
          <h2>Settings</h2>
          <button className="secondary-button" onClick={onOpenLogs}>
            <FileText size={16} />
            Logs
          </button>
          <button className="secondary-button" onClick={onClose}>
            Close
          </button>
        </header>
        <form
          className="settings-grid"
          onSubmit={(event) => {
            event.preventDefault();
            onSave({
              providers,
              activeProviderId,
              permissionMode,
              sandbox: sandboxSettings,
            });
          }}
        >
          <label>
            Backend URL
            <code>{platform?.backendUrl ?? "http://127.0.0.1:8787"}</code>
          </label>
          <label>
            Platform
            <code>{platform?.os ?? "browser"}</code>
          </label>
          <label>
            Permission
            <select
              value={permissionMode}
              onChange={(event) => {
                const nextMode = event.target.value as ExecutionPermissionMode;
                setPermissionMode(nextMode);
                setSandboxSettings((current) =>
                  nextMode === "full_access"
                    ? {
                        ...current,
                        sandboxMode: "danger-full-access",
                        enforcement: "disabled",
                        network: "allow",
                      }
                    : controlledSandboxSettings(current),
                );
              }}
            >
              <option value="approve">请求批准</option>
              <option value="auto">替我审批</option>
              <option value="full_access">完全访问权限</option>
            </select>
          </label>

          <div className="settings-sandbox-section">
            <div className="settings-providers-header">
              <h3>Sandbox</h3>
              <span>Applies to new tool calls immediately</span>
            </div>
            <div className="settings-sandbox-grid">
              <label>
                Access mode
                <select
                  value={sandboxSettings.sandboxMode}
                  onChange={(event) => {
                    const sandboxMode = event.target
                      .value as AppSettings["sandbox"]["sandboxMode"];
                    const danger = sandboxMode === "danger-full-access";
                    setSandboxSettings((current) => ({
                      ...current,
                      sandboxMode,
                      enforcement: danger
                        ? "disabled"
                        : current.enforcement === "disabled"
                          ? "enforce"
                          : current.enforcement,
                      network: danger ? "allow" : current.network,
                    }));
                  }}
                >
                  <option value="read-only">Read only</option>
                  <option value="workspace-write">Workspace write</option>
                  <option value="danger-full-access">Full system access</option>
                </select>
              </label>
              <label>
                OS enforcement
                <select
                  value={sandboxSettings.enforcement}
                  disabled={
                    sandboxSettings.sandboxMode === "danger-full-access"
                  }
                  onChange={(event) =>
                    setSandboxSettings((current) => ({
                      ...current,
                      enforcement: event.target
                        .value as AppSettings["sandbox"]["enforcement"],
                    }))
                  }
                >
                  <option value="enforce">Enforce</option>
                  <option value="best-effort">Best effort</option>
                  <option value="disabled">Disabled</option>
                </select>
              </label>
              <label>
                Network
                <select
                  value={sandboxSettings.network}
                  disabled={
                    sandboxSettings.sandboxMode === "danger-full-access"
                  }
                  onChange={(event) =>
                    setSandboxSettings((current) => ({
                      ...current,
                      network: event.target
                        .value as AppSettings["sandbox"]["network"],
                    }))
                  }
                >
                  <option value="deny">Deny</option>
                  <option value="inherit">Inherit</option>
                  <option value="allow">Allow</option>
                </select>
              </label>
              <label>
                Extra writable roots
                <textarea
                  rows={3}
                  placeholder="One absolute path per line"
                  value={sandboxSettings.writableRoots.join("\n")}
                  onChange={(event) =>
                    setSandboxSettings((current) => ({
                      ...current,
                      writableRoots: parsePathList(event.target.value),
                    }))
                  }
                />
              </label>
              <label>
                Extra readable paths
                <textarea
                  rows={3}
                  placeholder="One absolute path per line"
                  value={sandboxSettings.readPaths.join("\n")}
                  onChange={(event) =>
                    setSandboxSettings((current) => ({
                      ...current,
                      readPaths: parsePathList(event.target.value),
                    }))
                  }
                />
              </label>
            </div>
            {sandboxSettings.sandboxMode !== "danger-full-access" &&
              sandboxSettings.enforcement === "best-effort" && (
                <p className="settings-security-warning" role="status">
                  <ShieldAlert size={14} aria-hidden="true" />
                  Best effort may run commands without OS isolation when the
                  platform backend is unavailable. Use Enforce for security
                  testing.
                </p>
              )}
            {(sandboxSettings.sandboxMode === "danger-full-access" ||
              sandboxSettings.enforcement === "disabled") && (
              <p className="settings-security-warning" role="status">
                <ShieldAlert size={14} aria-hidden="true" />
                OS sandbox enforcement is disabled. Commands can access the full
                system and network allowed by the current user account.
              </p>
            )}
          </div>

          <div className="settings-providers-section">
            <div className="settings-providers-header">
              <h3>Providers</h3>
              <button
                type="button"
                className="secondary-button"
                onClick={addProvider}
              >
                <Plus size={14} /> Add Provider
              </button>
            </div>
            <div className="settings-providers-body">
              <div className="settings-provider-list">
                {providers.map((provider) => {
                  const health = providerHealth.find(
                    (h) => h.id === provider.id,
                  );
                  return (
                    <div
                      key={provider.id}
                      className={`settings-provider-item ${
                        provider.id === activeProviderId ? "active" : ""
                      } ${provider.id === editingProviderId ? "editing" : ""}`}
                    >
                      <div className="settings-provider-item-header">
                        <button
                          type="button"
                          className="settings-provider-select"
                          onClick={() => {
                            setActiveProviderId(provider.id);
                            setEditingProviderId(provider.id);
                          }}
                        >
                          <span className="settings-provider-name">
                            {provider.id === activeProviderId && (
                              <Check size={12} />
                            )}
                            {provider.id}
                          </span>
                        </button>
                        <span className="settings-provider-status">
                          {health?.status ?? "unknown"}
                        </span>
                        <button
                          type="button"
                          className="icon-button small"
                          disabled={providers.length <= 1}
                          onClick={() => removeProvider(provider.id)}
                        >
                          <Trash2 size={13} />
                        </button>
                      </div>
                    </div>
                  );
                })}
              </div>
              {editingProvider && (
                <div className="settings-provider-form">
                  <h4>Provider Details</h4>
                  <label>
                    ID
                    <input
                      value={editingProvider.id}
                      disabled
                      title="Provider ID 创建后保持稳定，用于关联安全存储中的凭据"
                    />
                  </label>
                  <label>
                    Provider Type
                    <select
                      value={editingProvider.kind}
                      onChange={(e) =>
                        updateProvider(
                          editingProvider.id,
                          "kind",
                          e.target.value as ProviderKind,
                        )
                      }
                    >
                      <option
                        value={
                          editingProvider.kind === "openai_responses"
                            ? "openai_responses"
                            : "openai_compatible"
                        }
                      >
                        OpenAI Compatible (auto)
                      </option>
                      <option value="anthropic">Anthropic Messages</option>
                      <option value="mock">Mock</option>
                    </select>
                  </label>
                  <label>
                    Base URL
                    <input
                      value={editingProvider.baseUrl}
                      onChange={(e) =>
                        updateProvider(
                          editingProvider.id,
                          "baseUrl",
                          e.target.value,
                        )
                      }
                    />
                  </label>
                  <label>
                    Model
                    <input
                      value={editingProvider.model}
                      onChange={(e) =>
                        updateProvider(
                          editingProvider.id,
                          "model",
                          e.target.value,
                        )
                      }
                    />
                  </label>
                  <div className="settings-provider-parameters">
                    <label>
                      Temperature
                      <input
                        type="number"
                        min="0"
                        max="2"
                        step="0.1"
                        value={editingProvider.temperature ?? ""}
                        placeholder="默认"
                        onChange={(event) =>
                          updateProvider(
                            editingProvider.id,
                            "temperature",
                            event.target.value
                              ? Number(event.target.value)
                              : null,
                          )
                        }
                      />
                    </label>
                    <label>
                      Max output tokens
                      <input
                        type="number"
                        min="1"
                        step="1"
                        value={editingProvider.maxOutputTokens ?? ""}
                        placeholder="Provider default"
                        onChange={(event) =>
                          updateProvider(
                            editingProvider.id,
                            "maxOutputTokens",
                            event.target.value
                              ? Number(event.target.value)
                              : null,
                          )
                        }
                      />
                    </label>
                    <label>
                      Context window
                      <input
                        type="number"
                        min="4096"
                        step="1024"
                        value={editingProvider.contextWindowTokens ?? ""}
                        onChange={(event) =>
                          updateProvider(
                            editingProvider.id,
                            "contextWindowTokens",
                            event.target.value
                              ? Number(event.target.value)
                              : null,
                          )
                        }
                      />
                    </label>
                    <label>
                      Reasoning effort
                      <select
                        value={editingProvider.reasoningEffort ?? ""}
                        onChange={(event) =>
                          updateProvider(
                            editingProvider.id,
                            "reasoningEffort",
                            (event.target.value || null) as
                              | "none"
                              | "minimal"
                              | "low"
                              | "medium"
                              | "high"
                              | "xhigh"
                              | "max"
                              | null,
                          )
                        }
                      >
                        <option value="">Provider default</option>
                        <option value="none">None</option>
                        <option value="minimal">Minimal</option>
                        <option value="low">Low</option>
                        <option value="medium">Medium</option>
                        <option value="high">High</option>
                        <option value="xhigh">Extra high</option>
                        <option value="max">Max</option>
                      </select>
                    </label>
                    <label>
                      Prompt cache key
                      <input
                        value={editingProvider.promptCacheKey ?? ""}
                        placeholder="Automatic per workspace"
                        onChange={(event) =>
                          updateProvider(
                            editingProvider.id,
                            "promptCacheKey",
                            event.target.value || null,
                          )
                        }
                      />
                    </label>
                    {editingProvider.kind === "openai_responses" && (
                      <>
                        <label>
                          Prompt cache policy
                          <select
                            value={editingProvider.promptCachePolicy ?? ""}
                            onChange={(event) =>
                              updateProvider(
                                editingProvider.id,
                                "promptCachePolicy",
                                (event.target.value || null) as NonNullable<
                                  ProviderSettings["promptCachePolicy"]
                                > | null,
                              )
                            }
                          >
                            <option value="">Automatic</option>
                            <option value="explicit_30m">
                              Explicit breakpoints (30m)
                            </option>
                            <option value="legacy_in_memory">
                              Legacy in-memory
                            </option>
                            <option value="legacy_24h">Legacy 24h</option>
                          </select>
                        </label>
                        <label>
                          Native compaction threshold
                          <input
                            type="number"
                            min="4096"
                            step="1024"
                            value={
                              editingProvider.responsesCompactionThresholdTokens ??
                              ""
                            }
                            placeholder="Disabled"
                            onChange={(event) =>
                              updateProvider(
                                editingProvider.id,
                                "responsesCompactionThresholdTokens",
                                event.target.value
                                  ? Number(event.target.value)
                                  : null,
                              )
                            }
                          />
                        </label>
                      </>
                    )}
                    <label>
                      <span>Rollout token budget</span>
                      <input
                        type="checkbox"
                        checked={Boolean(editingProvider.rolloutBudget)}
                        onChange={(event) =>
                          updateProvider(
                            editingProvider.id,
                            "rolloutBudget",
                            event.target.checked
                              ? {
                                  limitTokens: 100000,
                                  samplingTokenWeight: 1,
                                  prefillTokenWeight: 1,
                                }
                              : null,
                          )
                        }
                      />
                    </label>
                    {editingProvider.rolloutBudget ? (
                      <>
                        <label>
                          Weighted token limit
                          <input
                            type="number"
                            min="1"
                            step="1000"
                            value={editingProvider.rolloutBudget.limitTokens}
                            onChange={(event) =>
                              updateProvider(
                                editingProvider.id,
                                "rolloutBudget",
                                {
                                  ...editingProvider.rolloutBudget!,
                                  limitTokens: Number(event.target.value),
                                },
                              )
                            }
                          />
                        </label>
                        <label>
                          Output token weight
                          <input
                            type="number"
                            min="0"
                            step="0.1"
                            value={
                              editingProvider.rolloutBudget.samplingTokenWeight
                            }
                            onChange={(event) =>
                              updateProvider(
                                editingProvider.id,
                                "rolloutBudget",
                                {
                                  ...editingProvider.rolloutBudget!,
                                  samplingTokenWeight: Number(
                                    event.target.value,
                                  ),
                                },
                              )
                            }
                          />
                        </label>
                        <label>
                          Uncached input weight
                          <input
                            type="number"
                            min="0"
                            step="0.1"
                            value={
                              editingProvider.rolloutBudget.prefillTokenWeight
                            }
                            onChange={(event) =>
                              updateProvider(
                                editingProvider.id,
                                "rolloutBudget",
                                {
                                  ...editingProvider.rolloutBudget!,
                                  prefillTokenWeight: Number(
                                    event.target.value,
                                  ),
                                },
                              )
                            }
                          />
                        </label>
                      </>
                    ) : null}
                    <label>
                      <span>Parallel tool calls</span>
                      <input
                        type="checkbox"
                        checked={editingProvider.parallelToolCalls}
                        onChange={(event) =>
                          updateProvider(
                            editingProvider.id,
                            "parallelToolCalls",
                            event.target.checked,
                          )
                        }
                      />
                    </label>
                    {editingProvider.kind === "openai_responses" && (
                      <label>
                        <span>Stateful response continuation</span>
                        <input
                          type="checkbox"
                          checked={editingProvider.storeResponses}
                          onChange={(event) =>
                            updateProvider(
                              editingProvider.id,
                              "storeResponses",
                              event.target.checked,
                            )
                          }
                        />
                      </label>
                    )}
                  </div>
                  <div className="settings-provider-key-reference">
                    Credential reference:{" "}
                    <code>{editingProvider.apiKeySource}</code>
                  </div>
                  <div className="settings-provider-health-status">
                    {(() => {
                      const health = providerHealth.find(
                        (h) => h.id === editingProvider.id,
                      );
                      return (
                        <>
                          <span>Status: {health?.status ?? "unknown"}</span>
                          <span>
                            {health?.apiKeyConfigured
                              ? "key configured"
                              : "no key"}
                          </span>
                          <span>
                            {health?.usingMock
                              ? "mock active"
                              : "provider active"}
                          </span>
                        </>
                      );
                    })()}
                  </div>
                  <div className="settings-provider-actions">
                    <button
                      type="button"
                      className="secondary-button"
                      disabled={providerTest?.status === "testing"}
                      onClick={() =>
                        onTestProvider(editingProvider.id, providers)
                      }
                    >
                      {providerTest?.providerId === editingProvider.id &&
                      providerTest.status === "testing"
                        ? "Testing..."
                        : "Test connection"}
                    </button>
                    {providerTest?.providerId === editingProvider.id &&
                      providerTest.status === "complete" && (
                        <span className="settings-provider-test-result">
                          {providerTest.result?.reachable &&
                          providerTest.result.modelAvailable
                            ? `Connected${providerTest.result.latencyMs ? ` (${providerTest.result.latencyMs} ms)` : ""}`
                            : (providerTest.result?.error ??
                              "Connection failed")}
                        </span>
                      )}
                  </div>
                  {platform?.platform === "desktop" &&
                    secretSources?.keyring && (
                      <div className="settings-secret-section">
                        <label>
                          API key for {editingProvider.id}
                          <input
                            type="password"
                            autoComplete="off"
                            value={providerApiKey}
                            disabled={
                              !secretSources.keyring.encryptionAvailable
                            }
                            onChange={(event) =>
                              setProviderApiKey(event.target.value)
                            }
                          />
                        </label>
                        <div className="settings-provider-actions">
                          <button
                            type="button"
                            className="secondary-button"
                            disabled={
                              isSavingSecret ||
                              !secretSources.keyring.encryptionAvailable ||
                              !providerApiKey.trim()
                            }
                            onClick={() => {
                              const providerId = editingProvider.id;
                              const value = providerApiKey;
                              void onStoreProviderApiKey(
                                providerId,
                                value,
                              ).then((outcome) => {
                                if (!outcome.stored) return;
                                const { metadata } = outcome;
                                const nextProviders = providers.map(
                                  (provider) =>
                                    provider.id === providerId
                                      ? {
                                          ...provider,
                                          apiKeySource: metadata.envTarget,
                                          apiKeyConfigured: true,
                                        }
                                      : provider,
                                );
                                setProviders(nextProviders);
                                setProviderApiKey("");
                                onSave({
                                  providers: nextProviders,
                                  activeProviderId,
                                  permissionMode,
                                  sandbox: sandboxSettings,
                                });
                              });
                            }}
                          >
                            Store key
                          </button>
                          <button
                            type="button"
                            className="secondary-button"
                            disabled={
                              isSavingSecret ||
                              !editingProvider.apiKeyConfigured
                            }
                            onClick={() => {
                              const providerId = editingProvider.id;
                              void onDeleteProviderApiKey(providerId).then(
                                (outcome) => {
                                  if (!outcome.stored) return;
                                  const nextProviders = providers.map(
                                    (provider) =>
                                      provider.id === providerId
                                        ? {
                                            ...provider,
                                            apiKeyConfigured: false,
                                          }
                                        : provider,
                                  );
                                  setProviders(nextProviders);
                                  onSave({
                                    providers: nextProviders,
                                    activeProviderId,
                                    permissionMode,
                                    sandbox: sandboxSettings,
                                  });
                                },
                              );
                            }}
                          >
                            Remove key
                          </button>
                          <span className="settings-provider-test-result">
                            {editingProvider.apiKeyConfigured
                              ? "Encrypted in safeStorage and active"
                              : secretSources.keyring.status}
                          </span>
                        </div>
                      </div>
                    )}
                </div>
              )}
            </div>
          </div>

          <button className="primary-button" disabled={isSaving} type="submit">
            {isSaving ? "Saving..." : "Save"}
          </button>
        </form>
      </section>
    </div>
  );
}

function Sidebar({
  client,
  projects,
  threads,
  threadActivityStatuses,
  activeThreadId,
  activeProjectId,
  activeWorkspaceRemoteUrl,
  workspaceError,
  isPickingWorkspace,
  experienceMode,
  onExperienceModeChange,
  onSelect,
  onNew,
  onPickWorkspace,
  onCreateProject,
  onRemoveProject,
  onRenameProject,
  onToggleProjectPinned,
  onSelectProject,
  onOpenThreadWorkspace,
  onNewThreadForProject,
  onRenameThread,
  onRestoreThread,
  onOpenExtensions,
  onOpenTaskSearch,
  onSettings,
}: {
  client: ApiClient | null;
  projects: Project[];
  threads: Thread[];
  threadActivityStatuses: Record<string, ThreadActivityStatus>;
  activeThreadId: string | null;
  activeProjectId: string | null;
  activeWorkspaceRemoteUrl: string | null;
  workspaceError: string | null;
  isPickingWorkspace: boolean;
  experienceMode: ExperienceMode;
  onExperienceModeChange(mode: ExperienceMode): void;
  onSelect(id: string): void;
  onNew(): void;
  onPickWorkspace(): void;
  onCreateProject(name: string): Promise<Project | null>;
  onRemoveProject(project: Project): void;
  onRenameProject(project: Project): void;
  onToggleProjectPinned(project: Project): void;
  onSelectProject(project: Project): void;
  onOpenThreadWorkspace(workspaceRoot: string): void;
  onNewThreadForProject?(project: Project): void;
  onRenameThread(thread: Thread): void;
  onRestoreThread(thread: Thread): void;
  onOpenExtensions(): void;
  onOpenTaskSearch(): void;
  onSettings(): void;
}) {
  const [experienceMenuOpen, setExperienceMenuOpen] = useState(false);
  const [projectMenuOpen, setProjectMenuOpen] = useState(false);
  const [newProjectOpen, setNewProjectOpen] = useState(false);
  const [newProjectName, setNewProjectName] = useState("New project");
  const [isCreatingProject, setIsCreatingProject] = useState(false);
  const [expandedProjects, setExpandedProjects] = useState<Set<string>>(
    () => new Set(),
  );
  const [moreMenuProjectId, setMoreMenuProjectId] = useState<string | null>(
    null,
  );
  const [unassignedExpanded, setUnassignedExpanded] = useState(false);
  const [archivedExpanded, setArchivedExpanded] = useState(false);
  const [hoveredProject, setHoveredProject] =
    useState<ProjectHoverState | null>(null);
  const moreMenuRef = useDismissiblePopover(moreMenuProjectId !== null, () =>
    setMoreMenuProjectId(null),
  );
  const projectMenuRef = useDismissiblePopover(projectMenuOpen, () =>
    setProjectMenuOpen(false),
  );
  const experienceMenuRef = useDismissiblePopover(experienceMenuOpen, () =>
    setExperienceMenuOpen(false),
  );
  const unassignedThreads = threads.filter(
    (thread) => !thread.projectId && !thread.archivedAt,
  );
  const archivedThreads = threads.filter((thread) => thread.archivedAt);

  function toggleExpandedProject(projectId: string) {
    setExpandedProjects((prev) => {
      const next = new Set(prev);
      if (next.has(projectId)) {
        next.delete(projectId);
      } else {
        next.add(projectId);
      }
      return next;
    });
  }

  useEffect(() => {
    if (!newProjectOpen) return;
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setNewProjectOpen(false);
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [newProjectOpen]);

  async function createProject() {
    const name = newProjectName.trim();
    if (!name || isCreatingProject) return;
    setIsCreatingProject(true);
    const project = await onCreateProject(name);
    setIsCreatingProject(false);
    if (!project) return;
    setNewProjectOpen(false);
    setProjectMenuOpen(false);
    setNewProjectName("New project");
    onSelectProject(project);
  }

  return (
    <>
      <aside className="sidebar" id="workspace-sidebar">
        <div className="sidebar-brand-row">
          <div className="experience-mode-menu" ref={experienceMenuRef}>
            <button
              type="button"
              className="experience-mode-trigger"
              aria-label={`当前模式：${experienceMode === "work" ? "Work" : "Code"}`}
              aria-haspopup="menu"
              aria-expanded={experienceMenuOpen}
              onClick={() => setExperienceMenuOpen((current) => !current)}
            >
              {experienceMode === "work" ? (
                <BriefcaseBusiness size={15} aria-hidden="true" />
              ) : (
                <Code2 size={15} aria-hidden="true" />
              )}
              <span>{experienceMode === "work" ? "Work" : "Code"}</span>
              <ChevronDown
                className={experienceMenuOpen ? "open" : undefined}
                size={14}
                aria-hidden="true"
              />
            </button>
            {experienceMenuOpen && (
              <div className="tool-popover experience-mode-popover" role="menu">
                {(
                  [
                    { id: "work", label: "Work", icon: BriefcaseBusiness },
                    { id: "code", label: "Code", icon: Code2 },
                  ] as const
                ).map((option) => {
                  const Icon = option.icon;
                  const selected = option.id === experienceMode;
                  return (
                    <button
                      key={option.id}
                      type="button"
                      role="menuitemradio"
                      aria-checked={selected}
                      className={selected ? "active" : undefined}
                      onClick={() => {
                        onExperienceModeChange(option.id);
                        setExperienceMenuOpen(false);
                      }}
                    >
                      <Icon size={14} aria-hidden="true" />
                      <span>{option.label}</span>
                      {selected && <Check size={13} aria-hidden="true" />}
                    </button>
                  );
                })}
              </div>
            )}
          </div>
          <IconButton
            className="sidebar-icon-button"
            size="compact"
            title="搜索任务 (Ctrl+K)"
            aria-label="搜索任务"
            onClick={onOpenTaskSearch}
          >
            <Search size={15} aria-hidden="true" />
          </IconButton>
        </div>
        <nav className="primary-nav" aria-label="主要导航">
          <button onClick={onNew}>
            <FileText size={15} />
            <span>新建任务</span>
          </button>
          <button disabled title="已安排 · 未实现">
            <Clock3 size={15} />
            <span>已安排</span>
            <small>未实现</small>
          </button>
          <button onClick={onOpenExtensions} title="管理插件">
            <Plug size={15} />
            <span>插件</span>
            <small>插件</small>
          </button>
          <button disabled title="拉取请求 · 未实现">
            <GitPullRequest size={15} />
            <span>拉取请求</span>
            <small>未实现</small>
          </button>
        </nav>

        <div className="project-heading">
          <span>项目</span>
          <div className="sidebar-project-menu-wrap" ref={projectMenuRef}>
            <button
              className="sidebar-icon-button"
              disabled={isPickingWorkspace}
              onClick={() => setProjectMenuOpen((current) => !current)}
              title="添加项目"
              aria-label="添加项目"
              aria-expanded={projectMenuOpen}
            >
              {isPickingWorkspace ? (
                <Loader2 size={14} className="spin" />
              ) : (
                <SquarePen size={14} />
              )}
            </button>
            {projectMenuOpen && (
              <div className="tool-popover sidebar-project-popover" role="menu">
                <button
                  role="menuitem"
                  onClick={() => {
                    setNewProjectOpen(true);
                    setProjectMenuOpen(false);
                  }}
                >
                  <Plus size={14} />
                  <span>新建空白项目</span>
                </button>
                <button
                  role="menuitem"
                  onClick={() => {
                    onPickWorkspace();
                    setProjectMenuOpen(false);
                  }}
                >
                  <FolderOpen size={14} />
                  <span>使用现有文件夹</span>
                </button>
              </div>
            )}
          </div>
        </div>
        <div className="project-tree">
          {projects.map((project, projectIndex) => {
            const projectThreads = threads.filter(
              (thread) => thread.projectId === project.id && !thread.archivedAt,
            );
            const isActive = project.id === activeProjectId;
            const isExpanded = expandedProjects.has(project.id);
            const isMoreMenuOpen = moreMenuProjectId === project.id;
            const projectInfoId = `project-hover-card-${projectIndex}`;
            return (
              <section
                className={`project-node ${isActive ? "active" : ""}`}
                key={project.id}
                onMouseEnter={(event) => {
                  const bounds = event.currentTarget.getBoundingClientRect();
                  const cardWidth = 320;
                  const left = Math.min(
                    bounds.right + 8,
                    window.innerWidth - cardWidth - 8,
                  );
                  const remoteUrl =
                    project.id === activeProjectId
                      ? activeWorkspaceRemoteUrl
                      : null;
                  setHoveredProject({
                    id: projectInfoId,
                    name: project.name,
                    threadCount: projectThreads.length,
                    workspaceRoot: project.workspaceRoot,
                    pinned: project.pinned,
                    remoteUrl,
                    left: Math.max(8, left),
                    top: Math.max(
                      36,
                      Math.min(bounds.top, window.innerHeight - 174),
                    ),
                  });
                }}
                onMouseLeave={() => setHoveredProject(null)}
              >
                <div className="project-row">
                  <button
                    className="project-select"
                    title={project.workspaceRoot ?? project.name}
                    aria-label={`项目 ${project.name}`}
                    aria-describedby={projectInfoId}
                    onClick={() => {
                      toggleExpandedProject(project.id);
                      onSelectProject(project);
                    }}
                  >
                    {isExpanded ? (
                      <FolderOpen size={14} />
                    ) : (
                      <Folder size={14} />
                    )}
                    <span>{project.name}</span>
                  </button>
                  <div className="project-row-actions">
                    <div
                      className="project-menu-wrap"
                      ref={isMoreMenuOpen ? moreMenuRef : undefined}
                    >
                      <button
                        className="project-more"
                        aria-label={`菜单 ${project.name}`}
                        aria-expanded={isMoreMenuOpen}
                        onClick={() =>
                          setMoreMenuProjectId(
                            isMoreMenuOpen ? null : project.id,
                          )
                        }
                      >
                        <MoreHorizontal size={13} />
                      </button>
                      {isMoreMenuOpen && (
                        <div
                          className="tool-popover project-row-popover"
                          role="menu"
                        >
                          <button
                            role="menuitem"
                            disabled={!project.workspaceRoot}
                            onClick={() => {
                              if (project.workspaceRoot) {
                                onOpenThreadWorkspace(project.workspaceRoot);
                              }
                              setMoreMenuProjectId(null);
                            }}
                          >
                            <FolderOpen size={14} />
                            <span>在文件管理器中打开</span>
                          </button>
                          <button
                            role="menuitem"
                            onClick={() => {
                              onRenameProject(project);
                              setMoreMenuProjectId(null);
                            }}
                          >
                            <Pencil size={14} />
                            <span>重命名</span>
                          </button>
                          <button disabled title="Git 工作树管理尚未实现">
                            <GitFork size={14} />
                            <span>创建工作树</span>
                            <small>未实现</small>
                          </button>
                          <button
                            role="menuitem"
                            onClick={() => {
                              onRemoveProject(project);
                              setMoreMenuProjectId(null);
                            }}
                          >
                            <Archive size={14} />
                            <span>归档</span>
                          </button>
                        </div>
                      )}
                    </div>
                    <button
                      className="project-new-thread"
                      title="新建对话"
                      aria-label={`在 ${project.name} 中新建对话`}
                      onClick={() => {
                        onNewThreadForProject?.(project);
                      }}
                    >
                      <SquarePen size={13} />
                    </button>
                  </div>
                </div>
                {isExpanded && (
                  <div className="project-tasks">
                    {projectThreads.map((thread) => (
                      <SidebarThreadRow
                        active={thread.id === activeThreadId}
                        activityStatus={threadActivityStatuses[thread.id]}
                        client={client}
                        key={thread.id}
                        project={project}
                        thread={thread}
                        onSelect={() => onSelect(thread.id)}
                        onRename={() => onRenameThread(thread)}
                        onRemoveProject={onRemoveProject}
                        onToggleProjectPinned={onToggleProjectPinned}
                      />
                    ))}
                    {projectThreads.length === 0 && (
                      <span className="project-empty">无任务</span>
                    )}
                  </div>
                )}
              </section>
            );
          })}
          {unassignedThreads.length > 0 && (
            <section className="project-node">
              <div className="project-row">
                <button
                  className="project-select"
                  title="尚未归属到项目的任务"
                  onClick={() => setUnassignedExpanded((current) => !current)}
                >
                  {unassignedExpanded ? (
                    <FolderOpen size={14} />
                  ) : (
                    <Folder size={14} />
                  )}
                  <span>未归属任务 ({unassignedThreads.length})</span>
                </button>
              </div>
              {unassignedExpanded && (
                <div className="project-tasks">
                  {unassignedThreads.map((thread) => (
                    <SidebarThreadRow
                      active={thread.id === activeThreadId}
                      activityStatus={threadActivityStatuses[thread.id]}
                      client={client}
                      key={thread.id}
                      project={null}
                      thread={thread}
                      onSelect={() => onSelect(thread.id)}
                      onRename={() => onRenameThread(thread)}
                      onRemoveProject={onRemoveProject}
                      onToggleProjectPinned={onToggleProjectPinned}
                    />
                  ))}
                </div>
              )}
            </section>
          )}
          {archivedThreads.length > 0 && (
            <section className="project-node">
              <div className="project-row">
                <button
                  className="project-select"
                  title="查看可恢复的归档任务"
                  onClick={() => setArchivedExpanded((current) => !current)}
                >
                  <Archive size={14} />
                  <span>已归档 ({archivedThreads.length})</span>
                </button>
              </div>
              {archivedExpanded && (
                <div className="project-tasks">
                  {archivedThreads.map((thread) => (
                    <SidebarThreadRow
                      archived
                      active={false}
                      activityStatus={threadActivityStatuses[thread.id]}
                      client={client}
                      key={thread.id}
                      project={
                        projects.find(
                          (project) => project.id === thread.projectId,
                        ) ?? null
                      }
                      thread={thread}
                      onSelect={() => onRestoreThread(thread)}
                      onRename={() => onRenameThread(thread)}
                      onRemoveProject={onRemoveProject}
                      onToggleProjectPinned={onToggleProjectPinned}
                      onRestore={() => onRestoreThread(thread)}
                    />
                  ))}
                </div>
              )}
            </section>
          )}
          {projects.length === 0 && (
            <p className="workspace-empty">尚未打开项目</p>
          )}
          {workspaceError && (
            <p className="workspace-error">{workspaceError}</p>
          )}
        </div>

        <div className="sidebar-footer">
          <button
            className="sidebar-settings-button"
            title="设置"
            aria-label="设置"
            onClick={onSettings}
          >
            <Settings size={15} />
            <span className="opentopia-wordmark" aria-hidden="true">
              <span className="brand-open">Open</span>
              <span>Topia</span>
            </span>
          </button>
          <button disabled title="帮助 · 未实现" aria-label="帮助">
            <CircleHelp size={15} />
          </button>
        </div>
      </aside>
      {hoveredProject &&
        createPortal(
          <div
            className="project-hover-card"
            id={hoveredProject.id}
            role="tooltip"
            style={{ left: hoveredProject.left, top: hoveredProject.top }}
          >
            <header>
              <span>
                <Folder size={17} aria-hidden="true" />
                <strong>{hoveredProject.name}</strong>
              </span>
              <button
                disabled
                className={hoveredProject.pinned ? "active" : undefined}
                title={hoveredProject.pinned ? "已固定" : "未固定"}
                aria-label={hoveredProject.pinned ? "已固定" : "未固定"}
              >
                <Pin
                  size={14}
                  fill={hoveredProject.pinned ? "currentColor" : "none"}
                  aria-hidden="true"
                />
              </button>
            </header>
            <div className="project-hover-card__row">
              <MessageCircle size={15} aria-hidden="true" />
              <span>{hoveredProject.threadCount} 个对话串</span>
            </div>
            <div className="project-hover-card__divider" />
            <div className="project-hover-card__row">
              <GitFork size={15} aria-hidden="true" />
              <span title={hoveredProject.remoteUrl ?? undefined}>
                {hoveredProject.remoteUrl
                  ? compactRemoteLabel(hoveredProject.remoteUrl)
                  : "远程仓库信息未加载"}
              </span>
            </div>
            <div className="project-hover-card__row">
              <Folder size={15} aria-hidden="true" />
              <span title={hoveredProject.workspaceRoot ?? undefined}>
                {hoveredProject.workspaceRoot ?? "尚未选择工作区"}
              </span>
            </div>
          </div>,
          document.body,
        )}
      {newProjectOpen && (
        <div
          className="modal-backdrop project-modal-backdrop"
          role="presentation"
          onClick={() => setNewProjectOpen(false)}
        >
          <form
            className="project-name-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="project-name-title"
            onClick={(event) => event.stopPropagation()}
            onSubmit={(event) => {
              event.preventDefault();
              void createProject();
            }}
          >
            <header>
              <div>
                <h2 id="project-name-title">为项目命名</h2>
                <p>项目可以稍后再选择工作区。</p>
              </div>
              <button
                className="icon-button small"
                type="button"
                aria-label="关闭项目弹窗"
                onClick={() => setNewProjectOpen(false)}
              >
                <X size={14} />
              </button>
            </header>
            <input
              autoFocus
              aria-label="项目名称"
              value={newProjectName}
              onChange={(event) => setNewProjectName(event.target.value)}
            />
            <footer>
              <button
                className="secondary-button"
                type="button"
                onClick={() => setNewProjectOpen(false)}
              >
                取消
              </button>
              <button
                className="primary-button"
                type="submit"
                disabled={!newProjectName.trim() || isCreatingProject}
              >
                {isCreatingProject ? "保存中..." : "保存"}
              </button>
            </footer>
          </form>
        </div>
      )}
    </>
  );
}

function ThreadStatusIndicator({ status }: { status?: ThreadActivityStatus }) {
  if (!status) {
    return <span className="thread-row-status" aria-hidden="true" />;
  }

  const label =
    status === "processing"
      ? "处理中"
      : status === "succeeded"
        ? "已完成"
        : status === "failed"
          ? "失败"
          : status === "user_action"
            ? "需要手动操作"
            : "需要审批";

  return (
    <span
      className={`thread-row-status is-${status}`}
      role="img"
      aria-label={label}
      title={label}
    >
      {status === "processing" ? (
        <Loader2 size={14} className="spin" aria-hidden="true" />
      ) : status === "failed" ? (
        <CircleAlert size={14} aria-hidden="true" />
      ) : (
        <Circle size={9} fill="currentColor" aria-hidden="true" />
      )}
    </span>
  );
}

function SidebarThreadRow({
  client,
  thread,
  project,
  active,
  activityStatus,
  archived = false,
  onSelect,
  onRename,
  onRemoveProject,
  onToggleProjectPinned,
  onRestore,
}: {
  client: ApiClient | null;
  thread: Thread;
  project: Project | null;
  active: boolean;
  activityStatus?: ThreadActivityStatus;
  archived?: boolean;
  onSelect(): void;
  onRename(): void;
  onRemoveProject(project: Project): void;
  onToggleProjectPinned(project: Project): void;
  onRestore?(): void;
}) {
  const [menuOpen, setMenuOpen] = useState(false);
  const [hoverCardPosition, setHoverCardPosition] = useState<{
    left: number;
    top: number;
  } | null>(null);
  const [gitBranch, setGitBranch] = useState<string | null>(null);
  const [isGitBranchLoading, setIsGitBranchLoading] = useState(false);
  const [titleOverflow, setTitleOverflow] = useState({
    distance: 0,
    durationMs: 0,
  });
  const titleViewportRef = useRef<HTMLSpanElement>(null);
  const titleTextRef = useRef<HTMLSpanElement>(null);
  const branchRequestIdRef = useRef(0);
  const menuRef = useDismissiblePopover(menuOpen, () => setMenuOpen(false));
  const hoverCardId = `thread-hover-card-${thread.id}`;

  useEffect(() => {
    const viewport = titleViewportRef.current;
    const text = titleTextRef.current;
    if (!viewport || !text) return;

    const measure = () => {
      const distance = Math.max(
        0,
        Math.ceil(text.scrollWidth - viewport.clientWidth),
      );
      const durationMs =
        distance > 0 ? Math.min(8_000, 1_800 + distance * 24) : 0;
      setTitleOverflow((current) =>
        current.distance === distance && current.durationMs === durationMs
          ? current
          : { distance, durationMs },
      );
    };
    const frame = window.requestAnimationFrame(measure);
    const observer = new ResizeObserver(measure);
    observer.observe(viewport);
    observer.observe(text);
    return () => {
      window.cancelAnimationFrame(frame);
      observer.disconnect();
    };
  }, [thread.title]);

  const titleStyle =
    titleOverflow.distance > 0
      ? ({
          "--thread-title-scroll-distance": `${titleOverflow.distance}px`,
          "--thread-title-scroll-duration": `${titleOverflow.durationMs}ms`,
        } as CSSProperties)
      : undefined;

  function showHoverCard(target: HTMLButtonElement) {
    const bounds = target.getBoundingClientRect();
    const cardWidth = 320;
    const viewportMargin = 8;
    const cardHeight = 128;
    const left = Math.min(
      bounds.right + viewportMargin,
      window.innerWidth - cardWidth - viewportMargin,
    );
    setHoverCardPosition({
      left: Math.max(viewportMargin, left),
      top: Math.max(
        viewportMargin,
        Math.min(bounds.top, window.innerHeight - cardHeight - viewportMargin),
      ),
    });

    if (!client) {
      setGitBranch(null);
      setIsGitBranchLoading(false);
      return;
    }
    const requestId = branchRequestIdRef.current + 1;
    branchRequestIdRef.current = requestId;
    setGitBranch(null);
    setIsGitBranchLoading(true);
    void client
      .getGitStatus(thread.id)
      .then((status) => {
        if (branchRequestIdRef.current === requestId) {
          setGitBranch(status.branch);
          setIsGitBranchLoading(false);
        }
      })
      .catch(() => {
        if (branchRequestIdRef.current === requestId) {
          setGitBranch(null);
          setIsGitBranchLoading(false);
        }
      });
  }

  function hideHoverCard() {
    setHoverCardPosition(null);
  }

  return (
    <div className={`thread-row-wrap ${menuOpen ? "menu-open" : ""}`}>
      <button
        className={`thread-row ${active ? "active" : ""}`}
        onClick={onSelect}
        aria-label={thread.title}
        aria-describedby={hoverCardPosition ? hoverCardId : undefined}
        onMouseEnter={(event) => showHoverCard(event.currentTarget)}
        onMouseLeave={hideHoverCard}
        onFocus={(event) => showHoverCard(event.currentTarget)}
        onBlur={hideHoverCard}
      >
        <span
          className={`thread-title-viewport ${titleOverflow.distance > 0 ? "is-overflowing" : ""}`}
          ref={titleViewportRef}
        >
          <span
            className="thread-title-text"
            ref={titleTextRef}
            style={titleStyle}
          >
            {thread.title}
          </span>
        </span>
      </button>
      {hoverCardPosition &&
        createPortal(
          <div
            className="thread-hover-card"
            id={hoverCardId}
            role="tooltip"
            style={hoverCardPosition}
          >
            <header>
              <strong>{thread.title}</strong>
              <time dateTime={thread.updatedAt}>
                {formatRelativeThreadTime(thread.updatedAt)}
              </time>
            </header>
            <div className="thread-hover-card__row">
              <Folder size={16} aria-hidden="true" />
              <span>
                {project?.name ?? workspaceName(thread.workspaceRoot)}
              </span>
            </div>
            <div className="thread-hover-card__row">
              <GitBranch size={16} aria-hidden="true" />
              <span>
                {isGitBranchLoading
                  ? "正在读取 Git 分支"
                  : (gitBranch ?? "未检测到 Git 分支")}
              </span>
            </div>
          </div>,
          document.body,
        )}
      <ThreadStatusIndicator status={activityStatus} />
      <div className="thread-row-menu-wrap" ref={menuRef}>
        <button
          className="thread-row-more"
          type="button"
          aria-label={`任务菜单 ${thread.title}`}
          aria-expanded={menuOpen}
          onClick={() => setMenuOpen((current) => !current)}
        >
          <MoreHorizontal size={13} />
        </button>
        {menuOpen && (
          <div className="tool-popover thread-row-popover" role="menu">
            <button
              role="menuitem"
              onClick={() => {
                onRename();
                setMenuOpen(false);
              }}
            >
              <Pencil size={14} />
              <span>重命名</span>
            </button>
            {project ? (
              <>
                <button
                  role="menuitem"
                  onClick={() => {
                    onToggleProjectPinned(project);
                    setMenuOpen(false);
                  }}
                >
                  <Pin size={14} />
                  <span>{project.pinned ? "取消固定项目" : "固定项目"}</span>
                </button>
                <div className="tool-popover-separator" />
                <button
                  role="menuitem"
                  onClick={() => {
                    onRemoveProject(project);
                    setMenuOpen(false);
                  }}
                >
                  <X size={14} />
                  <span>从最近项目移除</span>
                </button>
              </>
            ) : (
              <>
                <button disabled title="此对话尚未归属到项目">
                  <Pin size={14} />
                  <span>固定项目</span>
                </button>
                <div className="tool-popover-separator" />
                <button disabled title="此对话尚未归属到项目">
                  <X size={14} />
                  <span>从最近项目移除</span>
                </button>
              </>
            )}
            {archived && onRestore && (
              <button
                role="menuitem"
                onClick={() => {
                  onRestore();
                  setMenuOpen(false);
                }}
              >
                <RotateCcw size={14} />
                <span>恢复到项目</span>
              </button>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

function ThreadHeader({
  thread,
  toolStageOpen,
  contextRailOpen,
  onOpenLocation,
  onOpenTool,
  onToggleContextRail,
  onToggleToolStage,
  onRename,
  onArchive,
}: {
  thread: Thread | null;
  toolStageOpen: boolean;
  contextRailOpen: boolean;
  onOpenLocation(): void;
  onOpenTool(kind: ToolTabKind): void;
  onToggleContextRail(): void;
  onToggleToolStage(): void;
  onRename(): void;
  onArchive(): void;
}) {
  const [menuOpen, setMenuOpen] = useState(false);
  const [taskMenuOpen, setTaskMenuOpen] = useState(false);
  const menuRef = useDismissiblePopover(menuOpen, () => setMenuOpen(false));
  const taskMenuRef = useDismissiblePopover(taskMenuOpen, () =>
    setTaskMenuOpen(false),
  );

  function selectTool(kind: ToolTabKind) {
    onOpenTool(kind);
    setMenuOpen(false);
  }

  return (
    <div className="thread-header">
      <div className="thread-heading">
        <Folder size={15} />
        <h1>{thread?.title ?? "新任务"}</h1>
        <div className="thread-heading-menu-wrap" ref={taskMenuRef}>
          <button
            className="thread-more"
            disabled={!thread}
            aria-label="任务菜单"
            aria-expanded={taskMenuOpen}
            onClick={() => {
              setTaskMenuOpen((current) => !current);
              setMenuOpen(false);
            }}
          >
            <MoreHorizontal size={15} />
          </button>
          {taskMenuOpen && thread && (
            <div className="tool-popover thread-heading-popover" role="menu">
              <button
                role="menuitem"
                onClick={() => {
                  onOpenLocation();
                  setTaskMenuOpen(false);
                }}
              >
                <FolderOpen size={14} />
                <span>在文件管理器中打开</span>
              </button>
              <button
                role="menuitem"
                onClick={() => {
                  onRename();
                  setTaskMenuOpen(false);
                }}
              >
                <Pencil size={14} />
                <span>重命名任务</span>
              </button>
              <button disabled title="Git 工作树管理尚未实现">
                <GitFork size={14} />
                <span>创建工作树</span>
                <small>未实现</small>
              </button>
              <button
                role="menuitem"
                onClick={() => {
                  onArchive();
                  setTaskMenuOpen(false);
                }}
              >
                <Archive size={14} />
                <span>归档任务</span>
              </button>
            </div>
          )}
        </div>
      </div>
      <div className="thread-actions">
        <div className="thread-tool-menu-wrap" ref={menuRef}>
          <button
            className="thread-tool-button"
            disabled={!thread}
            aria-expanded={menuOpen}
            aria-haspopup="menu"
            onClick={() => {
              setMenuOpen((current) => !current);
              setTaskMenuOpen(false);
            }}
          >
            <PanelRight size={14} />
            <span>打开位置</span>
            <ChevronDown size={12} />
          </button>
          {menuOpen && thread && (
            <div className="tool-popover thread-tool-popover" role="menu">
              <button
                role="menuitem"
                onClick={() => {
                  onOpenLocation();
                  setMenuOpen(false);
                }}
              >
                <FolderOpen size={14} />
                <span>文件管理器</span>
              </button>
              <button role="menuitem" onClick={() => selectTool("terminal")}>
                <TerminalSquare size={14} />
                <span>终端</span>
              </button>
              <button disabled title="VS Code 启动集成尚未实现">
                <FileCode2 size={14} />
                <span>VS Code</span>
                <small>未实现</small>
              </button>
              <button disabled title="Git Bash 启动集成尚未实现">
                <GitBranch size={14} />
                <span>Git Bash</span>
                <small>未实现</small>
              </button>
              <button disabled title="WSL 启动集成尚未实现">
                <Cloud size={14} />
                <span>WSL</span>
                <small>未实现</small>
              </button>
              <div className="tool-popover-separator" />
              <button role="menuitem" onClick={() => selectTool("files")}>
                <Folder size={14} />
                <span>文件工具</span>
              </button>
              <button role="menuitem" onClick={() => selectTool("diff")}>
                <GitBranch size={14} />
                <span>审查变更</span>
              </button>
            </div>
          )}
        </div>
        <IconButton
          className={`context-rail-toggle ${contextRailOpen ? "is-active" : ""}`}
          size="compact"
          variant="quiet"
          aria-label={contextRailOpen ? "折叠环境信息" : "展开环境信息"}
          aria-controls="workspace-context-rail"
          aria-expanded={contextRailOpen}
          disabled={!thread}
          title={contextRailOpen ? "折叠环境信息" : "展开环境信息"}
          onClick={onToggleContextRail}
        >
          <SlidersHorizontal size={15} aria-hidden="true" />
        </IconButton>
        {!toolStageOpen ? (
          <IconButton
            className="tool-stage-toggle"
            size="compact"
            variant="quiet"
            aria-label="展开工具窗口"
            aria-controls="workspace-right-panel"
            aria-expanded={false}
            title="展开工具窗口"
            onClick={onToggleToolStage}
          >
            <PanelRightOpen size={15} aria-hidden="true" />
          </IconButton>
        ) : null}
      </div>
    </div>
  );
}

function GoalStrip({
  snapshot,
  isRunning,
  action,
  onRun,
  onPause,
  onCancel,
}: {
  snapshot: GoalSnapshot;
  isRunning: boolean;
  action: GoalStatus | "run" | null;
  onRun(): void;
  onPause(): void;
  onCancel(): void;
}) {
  const completed = snapshot.tasks.filter(
    (task) => task.status === "succeeded",
  ).length;
  const resolved = snapshot.tasks.filter((task) =>
    ["succeeded", "deferred", "blocked", "cancelled", "failed"].includes(
      task.status,
    ),
  ).length;
  const total = snapshot.tasks.length;
  const progress = total ? Math.round((completed / total) * 100) : 0;
  const succeededIds = new Set(
    snapshot.tasks
      .filter((task) => task.status === "succeeded")
      .map((task) => task.stepId),
  );
  let currentTaskIndex = snapshot.tasks.findIndex(
    (task) => task.status === "running",
  );
  if (currentTaskIndex < 0) {
    currentTaskIndex = snapshot.tasks.findIndex(
      (task) =>
        task.status === "pending" &&
        task.dependencies.every((dependency) => succeededIds.has(dependency)),
    );
  }
  const terminal = ["completed", "cancelled", "failed"].includes(
    snapshot.goal.status,
  );
  const canRun =
    !isRunning &&
    ["ready", "active", "paused", "blocked"].includes(snapshot.goal.status);
  return (
    <section className={`goal-strip is-${snapshot.goal.status}`}>
      <details open>
        <summary>
          <span className="goal-strip-icon" aria-hidden="true">
            <Target size={15} />
          </span>
          <span className="goal-strip-objective">
            {snapshot.goal.objective}
          </span>
          <span className={`goal-status is-${snapshot.goal.status}`}>
            {goalStatusLabel(snapshot.goal.status)}
          </span>
          {total ? (
            <span className="goal-count">
              {currentTaskIndex >= 0
                ? `第 ${currentTaskIndex + 1}/${total} 步`
                : `${resolved}/${total} 已处理`}
            </span>
          ) : null}
        </summary>
        <div className="goal-strip-body">
          <div
            className="goal-progress"
            role="progressbar"
            aria-label="目标进度"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={progress}
          >
            <span style={{ width: `${progress}%` }} />
          </div>
          {snapshot.tasks.length ? (
            <ol className="goal-task-list">
              {snapshot.tasks.map((task) => (
                <li className={`is-${task.status}`} key={task.stepId}>
                  <span className="goal-task-state" aria-hidden="true" />
                  <span className="goal-task-content">
                    <span>{task.title}</span>
                    {task.statusReason ? (
                      <small>{task.statusReason}</small>
                    ) : null}
                  </span>
                  {task.attemptCount ? (
                    <small className="goal-task-attempts">
                      {task.attemptCount}x
                    </small>
                  ) : null}
                </li>
              ))}
            </ol>
          ) : null}
          {!terminal ? (
            <div className="goal-actions">
              {canRun ? (
                <button
                  type="button"
                  disabled={Boolean(action)}
                  onClick={onRun}
                >
                  {action === "run" ? (
                    <Loader2 size={14} className="spin" />
                  ) : (
                    <Zap size={14} />
                  )}
                  <span>
                    {snapshot.goal.status === "ready" ? "启动" : "继续"}
                  </span>
                </button>
              ) : null}
              {snapshot.goal.status === "active" && isRunning ? (
                <button
                  type="button"
                  disabled={Boolean(action)}
                  onClick={onPause}
                >
                  {action === "paused" ? (
                    <Loader2 size={14} className="spin" />
                  ) : (
                    <Pause size={14} />
                  )}
                  <span>暂停</span>
                </button>
              ) : null}
              <button
                className="goal-cancel-button"
                type="button"
                title="取消目标"
                aria-label="取消目标"
                disabled={Boolean(action)}
                onClick={onCancel}
              >
                {action === "cancelled" ? (
                  <Loader2 size={14} className="spin" />
                ) : (
                  <X size={14} />
                )}
              </button>
            </div>
          ) : null}
        </div>
      </details>
    </section>
  );
}

function goalStatusLabel(status: GoalStatus): string {
  const labels: Record<GoalStatus, string> = {
    draft: "规划中",
    ready: "待启动",
    active: "执行中",
    paused: "已暂停",
    completed: "已完成",
    blocked: "受阻",
    cancelled: "已取消",
    failed: "失败",
  };
  return labels[status];
}

const initialRenderedMessageCount = 12;
const messageRenderBatchSize = 12;

function ConversationLoadingState() {
  return (
    <section
      className="conversation-loading"
      role="status"
      aria-label="正在加载"
      aria-live="polite"
    >
      <div className="conversation-loading__content">
        <span className="conversation-loading__wordmark" aria-hidden="true">
          <span className="conversation-loading__brand-open">Open</span>
          <span>Topia</span>
        </span>
      </div>
    </section>
  );
}

function ConversationLoadErrorState({
  error,
  onRetry,
}: {
  error: string;
  onRetry(): void;
}) {
  return (
    <section className="conversation-load-error" role="alert">
      <div className="conversation-load-error__content">
        <strong>无法加载会话内容</strong>
        <p>{error}</p>
        <Button variant="secondary" size="compact" onClick={onRetry}>
          重试
        </Button>
      </div>
    </section>
  );
}

function MessageList({
  messages,
  events,
  activeTurnId,
  pendingTurnFeedback,
  undoingTurnId,
  threadId,
  artifacts,
  onOpenArtifact,
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
      if (event.turnId && event.payload.type === "turn_changes_recorded") {
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
        aria-busy={hasPendingMessages || showPendingTurnStatus}
        onCopy={trimCopiedSelection}
      >
        <div
          className={`message-list-content ${
            visibleMessages.length === 0 && !showPendingTurnStatus
              ? "is-empty"
              : ""
          }`.trim()}
          ref={messageListContentRef}
        >
          {visibleMessages.length === 0 && !showPendingTurnStatus ? (
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
                    message={message}
                    threadId={threadId}
                    artifacts={artifacts}
                    onOpenArtifact={onOpenArtifact}
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
          {showPendingTurnStatus && pendingTurnFeedback ? (
            <PendingTurnStatus
              key={pendingTurnFeedback.startedAt}
              phase={pendingTurnFeedback.phase}
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

/**
 * Hands the clipboard the text that was actually selected. Chromium serializes
 * a selection block by block, so a drag that lands a hair past the last glyph
 * of a message still closes that block and opens the next one, and the paste
 * arrives with a blank line or two glued to the end. The rewrite is skipped
 * when nothing needs trimming, which keeps the rich-text flavor intact for the
 * copies that were already clean.
 */
function trimCopiedSelection(event: ReactClipboardEvent<HTMLDivElement>) {
  const selected = window.getSelection()?.toString() ?? "";
  const trimmed = normalizeCopiedText(selected);
  if (!trimmed || trimmed === selected) return;
  event.clipboardData.setData("text/plain", trimmed);
  event.preventDefault();
}

const MessageBubble = memo(function MessageBubble({
  message,
  threadId,
  artifacts,
  onOpenArtifact,
  onOpenMarkdownLink,
}: {
  message: Message;
  threadId: string;
  artifacts: ArtifactDescriptor[];
  onOpenArtifact(artifactId: string): void;
  onOpenMarkdownLink(href: string): void;
}) {
  const visibleParts = message.parts.filter(
    (part) =>
      part.type !== "turn_context" &&
      part.type !== "tool_call" &&
      part.type !== "tool_result",
  );
  if (visibleParts.length === 0) return null;

  return (
    <article className={`message ${message.role}`}>
      <div className="message-body">
        {visibleParts.map((part, index) => (
          <MessagePartView
            key={index}
            messageId={message.id}
            part={part}
            role={message.role}
            threadId={threadId}
            artifacts={artifacts}
            onOpenArtifact={onOpenArtifact}
            onOpenMarkdownLink={onOpenMarkdownLink}
          />
        ))}
      </div>
    </article>
  );
});

function MessagePartView({
  messageId,
  part,
  role,
  threadId,
  artifacts,
  onOpenArtifact,
  onOpenMarkdownLink,
}: {
  messageId: string;
  part: MessagePart;
  role: Message["role"];
  threadId: string;
  artifacts: ArtifactDescriptor[];
  onOpenArtifact(artifactId: string): void;
  onOpenMarkdownLink(href: string): void;
}) {
  if (part.type === "image") {
    return <InlineImageMessagePart part={part} />;
  }
  if (part.type === "text") {
    const refs = artifactReferencesFromText(part.text);
    return (
      <>
        {role === "assistant" ? (
          <MarkdownContent
            className="message-markdown"
            onOpenLink={onOpenMarkdownLink}
            renderTrace={{
              channel: "assistant",
              threadId,
              messageId,
            }}
            text={part.text}
          />
        ) : (
          <p className="message-text">{part.text}</p>
        )}
        <MessageArtifactLinks
          refs={refs}
          threadId={threadId}
          artifacts={artifacts}
          onOpenArtifact={onOpenArtifact}
        />
      </>
    );
  }
  if (part.type === "error")
    return <p className="message-error">{part.message}</p>;
  if (part.type === "file_ref") return <code>{part.path}</code>;
  if (part.type === "source_ref") {
    return (
      <button
        className="message-source-reference"
        type="button"
        title={part.source.path}
        onClick={() => void openPath(part.source.path)}
      >
        <ContextSourceIcon extension={fileExtension(part.source.name)} />
        <span>{part.source.name}</span>
        <small>{formatBytes(part.source.bytes)}</small>
      </button>
    );
  }
  if (part.type === "skill_ref") {
    return (
      <button
        className="message-source-reference is-skill"
        type="button"
        title={part.skill.description || part.skill.path}
        onClick={() => void openPath(part.skill.path)}
      >
        <Plug size={12} />
        <span>{part.skill.name}</span>
        <small>Skill</small>
      </button>
    );
  }
  return null;
}

function InlineImageMessagePart({
  part,
}: {
  part: Extract<MessagePart, { type: "image" }>;
}) {
  const previewUrl = useMemo(
    () =>
      URL.createObjectURL(
        new Blob([new Uint8Array(part.data)], { type: part.contentType }),
      ),
    [part.contentType, part.data],
  );

  useEffect(() => () => URL.revokeObjectURL(previewUrl), [previewUrl]);

  return (
    <div className="message-inline-image">
      <img src={previewUrl} alt={part.name || "已发送图片"} />
    </div>
  );
}

function MessageArtifactLinks({
  refs,
  artifacts,
  onOpenArtifact,
}: {
  refs: ArtifactReference[];
  threadId: string;
  artifacts: ArtifactDescriptor[];
  onOpenArtifact(artifactId: string): void;
}) {
  if (!refs.length) return null;
  return (
    <div className="message-artifact-links">
      {refs.map((ref) => {
        const descriptor = artifacts.find((artifact) => artifact.id === ref.id);
        return (
          <button
            className="artifact-reference-button"
            key={ref.id}
            type="button"
            title={ref.id}
            onClick={() => onOpenArtifact(ref.id)}
          >
            <ExternalLink size={12} />
            <span>{descriptor?.kind ?? ref.kind ?? "artifact"}</span>
            <small>
              {descriptor?.bytes
                ? formatBytes(descriptor.bytes)
                : ref.bytes
                  ? formatBytes(ref.bytes)
                  : "load"}
            </small>
          </button>
        );
      })}
    </div>
  );
}

function ComposerTaskPlan({ plan }: { plan: TaskPlan }) {
  const [expanded, setExpanded] = useState(false);
  const completedIds = useMemo(
    () =>
      new Set(
        plan.steps
          .filter((step) => step.status === "completed")
          .map((step) => step.id),
      ),
    [plan.steps],
  );
  const currentStepIndex = useMemo(() => {
    const inProgressIndex = plan.steps.findIndex(
      (step) => step.status === "in_progress",
    );
    if (inProgressIndex >= 0) return inProgressIndex;
    return plan.steps.findIndex(
      (step) =>
        step.status === "pending" &&
        step.dependencies.every((dependency) => completedIds.has(dependency)),
    );
  }, [completedIds, plan.steps]);
  const resolvedCount = plan.steps.filter((step) =>
    ["completed", "deferred", "blocked", "cancelled"].includes(step.status),
  ).length;
  const currentStep =
    currentStepIndex >= 0 ? plan.steps[currentStepIndex] : undefined;
  const progressLabel = currentStep
    ? `第 ${currentStepIndex + 1}/${plan.steps.length} 步`
    : `${resolvedCount}/${plan.steps.length} 已处理`;

  useEffect(() => {
    setExpanded(false);
  }, [plan.goalId]);

  if (plan.steps.length === 0) return null;

  return (
    <section className={`composer-plan ${expanded ? "is-expanded" : ""}`}>
      <button
        className="composer-plan-summary"
        type="button"
        aria-expanded={expanded}
        aria-controls="composer-plan-steps"
        onClick={() => setExpanded((current) => !current)}
      >
        <ListTodo size={15} aria-hidden="true" />
        <span className="composer-plan-current">
          {currentStep?.title || currentStep?.step || "任务清单"}
        </span>
        <span className="composer-plan-count">{progressLabel}</span>
        <ChevronDown
          className="composer-plan-chevron"
          size={14}
          aria-hidden="true"
        />
      </button>
      {expanded ? (
        <div className="composer-plan-body" id="composer-plan-steps">
          <ol className="composer-plan-list">
            {plan.steps.map((step, index) => (
              <li
                className={`is-${step.status} ${index === currentStepIndex ? "is-current" : ""}`}
                data-status={step.status}
                key={step.id}
              >
                <span className="composer-plan-step-icon" aria-hidden="true">
                  <ComposerPlanStepIcon status={step.status} />
                </span>
                <span className="composer-plan-step-copy">
                  <span>{step.title || step.step || step.id}</span>
                  {step.statusReason ? (
                    <small>{step.statusReason}</small>
                  ) : null}
                </span>
                {index === currentStepIndex ? (
                  <span className="composer-plan-step-marker">当前</span>
                ) : null}
              </li>
            ))}
          </ol>
        </div>
      ) : null}
    </section>
  );
}

function ComposerPlanStepIcon({
  status,
}: {
  status: TaskPlan["steps"][number]["status"];
}) {
  if (status === "completed")
    return <span className="composer-plan-complete" />;
  if (status === "in_progress") return <span className="composer-plan-flow" />;
  if (status === "blocked") return <AlertCircle size={13} />;
  if (status === "cancelled") return <X size={13} />;
  if (status === "deferred") return <Clock3 size={13} />;
  return <Circle size={11} />;
}

const MAX_COMPOSER_IMAGES = 10;
const MAX_COMPOSER_IMAGE_BYTES = 25 * 1024 * 1024;

type ComposerImageAttachment = InlineImageAttachment & {
  id: string;
  previewUrl: string;
};

function fileExtension(name: string): string {
  const baseName = name.split(/[\\/]/).pop() ?? name;
  const dotIndex = baseName.lastIndexOf(".");
  return dotIndex > 0 ? baseName.slice(dotIndex + 1) : "";
}

function ContextSourceIcon({ extension }: { extension: string }) {
  const value = extension.replace(/^\./, "").toLocaleLowerCase();
  if (["png", "jpg", "jpeg", "gif", "webp", "bmp"].includes(value)) {
    return <FileImage size={12} aria-hidden="true" />;
  }
  if (["csv", "tsv", "xls", "xlsx", "xlsm", "xlsb", "ods"].includes(value)) {
    return <Table2 size={12} aria-hidden="true" />;
  }
  if (["ppt", "pptx", "pps", "ppsx", "pot", "potx", "odp"].includes(value)) {
    return <Presentation size={12} aria-hidden="true" />;
  }
  if (["json", "jsonc", "jsonl"].includes(value)) {
    return <FileJson size={12} aria-hidden="true" />;
  }
  if (
    [
      "rs",
      "ts",
      "tsx",
      "js",
      "jsx",
      "mjs",
      "cjs",
      "c",
      "h",
      "cc",
      "cpp",
      "hpp",
      "py",
      "go",
      "java",
      "kt",
      "swift",
      "rb",
      "php",
      "sh",
      "ps1",
      "bat",
      "cmd",
      "sql",
      "graphql",
      "gql",
      "proto",
      "diff",
      "patch",
      "xml",
      "html",
      "htm",
      "css",
      "scss",
      "less",
      "yaml",
      "yml",
      "toml",
    ].includes(value)
  ) {
    return <FileCode2 size={12} aria-hidden="true" />;
  }
  if (
    [
      "pdf",
      "doc",
      "docx",
      "odt",
      "rtf",
      "md",
      "mdx",
      "txt",
      "log",
      "ini",
      "conf",
      "config",
      "properties",
    ].includes(value)
  ) {
    return <FileText size={12} aria-hidden="true" />;
  }
  return <File size={12} aria-hidden="true" />;
}

function Composer({
  autoFocus = false,
  value,
  taskPlan,
  isSending,
  isRunning,
  isCancelling,
  queuedMessageCount = 0,
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
  value: string;
  taskPlan?: TaskPlan | null;
  isSending: boolean;
  isRunning: boolean;
  isCancelling: boolean;
  queuedMessageCount?: number;
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
  const [openMenu, setOpenMenu] = useState<
    "actions" | "permission" | "model" | "workspace" | "environment" | null
  >(null);
  const closeMenus = () => {
    setOpenMenu(null);
  };
  const popoverRef = useDismissiblePopover(Boolean(openMenu), closeMenus);
  const [draft, setDraft] = useState(value);
  const [imageAttachments, setImageAttachments] = useState<
    ComposerImageAttachment[]
  >([]);
  const [previewIndex, setPreviewIndex] = useState<number | null>(null);
  const [isDraggingFiles, setIsDraggingFiles] = useState(false);
  const dragDepthRef = useRef(0);
  const imageAttachmentsRef = useRef(imageAttachments);

  useEffect(() => {
    imageAttachmentsRef.current = imageAttachments;
  }, [imageAttachments]);

  useEffect(
    () => () => {
      imageAttachmentsRef.current.forEach((attachment) =>
        URL.revokeObjectURL(attachment.previewUrl),
      );
    },
    [],
  );

  useEffect(() => {
    setDraft(value);
  }, [value]);

  const submitDraft = async () => {
    if (isSending) return;
    const submittedValue = draft;
    const submittedAttachments = imageAttachments.map(
      ({ id: _id, previewUrl: _previewUrl, ...attachment }) => attachment,
    );
    const accepted = await onSubmit(submittedValue, submittedAttachments);
    if (!accepted) return;
    imageAttachments.forEach((attachment) =>
      URL.revokeObjectURL(attachment.previewUrl),
    );
    setImageAttachments([]);
    setPreviewIndex(null);
    setDraft("");
    onChange("");
  };

  async function handlePaste(event: ReactClipboardEvent<HTMLTextAreaElement>) {
    const items = Array.from(event.clipboardData.items).filter(
      (item) => item.kind === "file" && item.type.startsWith("image/"),
    );
    if (items.length === 0) return;

    event.preventDefault();
    const remaining = Math.max(
      0,
      MAX_COMPOSER_IMAGES - imageAttachments.length,
    );
    const next: ComposerImageAttachment[] = [];
    for (const item of items.slice(0, remaining)) {
      const file = item.getAsFile();
      if (!file || file.size > MAX_COMPOSER_IMAGE_BYTES) continue;
      const data = Array.from(new Uint8Array(await file.arrayBuffer()));
      next.push({
        id: `pasted-image-${Date.now()}-${next.length}`,
        name: file.name || `pasted-image-${next.length + 1}.png`,
        contentType: file.type || "image/png",
        data,
        previewUrl: URL.createObjectURL(file),
      });
    }
    if (next.length > 0) {
      setImageAttachments((current) => [...current, ...next]);
    }
  }

  function removeImageAttachment(id: string) {
    setImageAttachments((current) => {
      const removedIndex = current.findIndex(
        (attachment) => attachment.id === id,
      );
      const removed = removedIndex >= 0 ? current[removedIndex] : undefined;
      if (removed) URL.revokeObjectURL(removed.previewUrl);
      const next = current.filter((attachment) => attachment.id !== id);
      if (previewIndex !== null) {
        if (next.length === 0) {
          setPreviewIndex(null);
        } else if (removedIndex < previewIndex) {
          setPreviewIndex(previewIndex - 1);
        } else if (previewIndex >= next.length) {
          setPreviewIndex(next.length - 1);
        }
      }
      return next;
    });
  }

  function handleDragEnter(event: ReactDragEvent<HTMLDivElement>) {
    if (!Array.from(event.dataTransfer.types).includes("Files")) return;
    event.preventDefault();
    dragDepthRef.current += 1;
    setIsDraggingFiles(true);
  }

  function handleDragOver(event: ReactDragEvent<HTMLDivElement>) {
    if (!Array.from(event.dataTransfer.types).includes("Files")) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "copy";
  }

  function handleDragLeave() {
    dragDepthRef.current = Math.max(0, dragDepthRef.current - 1);
    if (dragDepthRef.current === 0) setIsDraggingFiles(false);
  }

  function handleDrop(event: ReactDragEvent<HTMLDivElement>) {
    if (!Array.from(event.dataTransfer.types).includes("Files")) return;
    event.preventDefault();
    dragDepthRef.current = 0;
    setIsDraggingFiles(false);
    const files = Array.from(event.dataTransfer.files);
    if (files.length > 0) void onAddContextSources(files);
  }

  const hasSendableContent = Boolean(
    draft.trim() ||
    imageAttachments.length > 0 ||
    contextSources.length > 0 ||
    selectedSkillIds.length > 0,
  );

  return (
    <div className="composer-shell">
      {taskPlan ? <ComposerTaskPlan plan={taskPlan} /> : null}
      <div
        className={`composer ${workspaceRoot || projectName ? "has-context" : ""} ${contextSources.length || imageAttachments.length || selectedSkillIds.length ? "has-sources" : ""} ${isDraggingFiles ? "is-dragging-files" : ""}`}
        ref={popoverRef}
        onDragEnter={handleDragEnter}
        onDragOver={handleDragOver}
        onDragLeave={handleDragLeave}
        onDrop={handleDrop}
      >
        {(workspaceRoot || projectName) && (
          <div className="composer-context">
            <div className="composer-menu-wrap">
              <button
                className="composer-context-button"
                type="button"
                title={workspaceRoot ?? projectName ?? "项目"}
                aria-expanded={openMenu === "workspace"}
                onClick={() =>
                  setOpenMenu((current) =>
                    current === "workspace" ? null : "workspace",
                  )
                }
              >
                <Folder size={12} />
                <span>{projectName ?? workspaceName(workspaceRoot ?? "")}</span>
                <ChevronDown size={11} />
              </button>
              {openMenu === "workspace" && (
                <div className="tool-popover workspace-popover" role="menu">
                  <div className="tool-popover-note">
                    <strong>选择工作区</strong>
                    <span>当前任务将使用所选文件夹</span>
                  </div>
                  {projects
                    .filter((project) => project.workspaceRoot)
                    .map((project) => (
                      <button
                        key={project.id}
                        role="menuitemradio"
                        aria-checked={project.workspaceRoot === workspaceRoot}
                        onClick={() => {
                          onSelectProject(project.id);
                          setOpenMenu(null);
                        }}
                      >
                        {project.workspaceRoot === workspaceRoot ? (
                          <Check size={13} />
                        ) : (
                          <Folder size={13} />
                        )}
                        <span>{project.name}</span>
                      </button>
                    ))}
                  <div className="tool-popover-separator" />
                  <button
                    role="menuitem"
                    onClick={() => {
                      onPickWorkspace();
                      setOpenMenu(null);
                    }}
                  >
                    <FolderOpen size={14} />
                    <span>选择其他文件夹</span>
                  </button>
                </div>
              )}
            </div>
            <div className="composer-menu-wrap">
              {launchMode && onChangeLaunchMode ? (
                <>
                  <button
                    className="composer-context-button"
                    type="button"
                    aria-label="选择启动模式"
                    aria-expanded={openMenu === "environment"}
                    onClick={() =>
                      setOpenMenu((current) =>
                        current === "environment" ? null : "environment",
                      )
                    }
                  >
                    {launchMode === "local" ? (
                      <Laptop size={12} />
                    ) : (
                      <GitFork size={12} />
                    )}
                    <span>{newTaskLaunchModeLabel(launchMode)}</span>
                    <ChevronDown size={11} />
                  </button>
                  {openMenu === "environment" && (
                    <div
                      className="tool-popover launch-mode-popover"
                      role="menu"
                    >
                      <div className="tool-popover-note">
                        <strong>启动模式</strong>
                        <span>选择新任务使用的工作区方式</span>
                      </div>
                      <button
                        className={launchMode === "local" ? "active" : ""}
                        role="menuitemradio"
                        aria-checked={launchMode === "local"}
                        onClick={() => {
                          onChangeLaunchMode("local");
                          setOpenMenu(null);
                        }}
                      >
                        <Laptop size={14} />
                        <span>在本地处理</span>
                        {launchMode === "local" && <Check size={13} />}
                      </button>
                      <button
                        className={
                          launchMode === "new_worktree" ? "active" : ""
                        }
                        role="menuitemradio"
                        aria-checked={launchMode === "new_worktree"}
                        title="线程级工作树创建尚未接入"
                        onClick={() => {
                          onChangeLaunchMode("new_worktree");
                          setOpenMenu(null);
                        }}
                      >
                        <GitFork size={14} />
                        <span>新工作树</span>
                        <small>内部未实现</small>
                      </button>
                      <button
                        disabled
                        role="menuitem"
                        title="云端任务执行尚未实现"
                      >
                        <Cloud size={14} />
                        <span>发送至云端</span>
                        <small>未实现</small>
                      </button>
                    </div>
                  )}
                </>
              ) : (
                <>
                  <button
                    className="composer-context-button"
                    type="button"
                    aria-expanded={openMenu === "environment"}
                    onClick={() =>
                      setOpenMenu((current) =>
                        current === "environment" ? null : "environment",
                      )
                    }
                  >
                    <TerminalSquare size={12} />
                    <span>{sandboxModeLabel(sandboxMode)}</span>
                    <ChevronDown size={11} />
                  </button>
                  {openMenu === "environment" && (
                    <div
                      className="tool-popover environment-popover"
                      role="menu"
                    >
                      {sandboxModeOptions.map((option) => (
                        <button
                          className={
                            sandboxMode === option.value ? "active" : ""
                          }
                          key={option.value}
                          role="menuitemradio"
                          aria-checked={sandboxMode === option.value}
                          onClick={() => {
                            onChangeSandboxMode(option.value);
                            setOpenMenu(null);
                          }}
                        >
                          {sandboxMode === option.value ? (
                            <Check size={13} />
                          ) : (
                            <span className="menu-icon-spacer" />
                          )}
                          <span>{option.label}</span>
                          <small>{option.detail}</small>
                        </button>
                      ))}
                      <div className="tool-popover-separator" />
                      <button disabled title="Git 工作树创建尚未实现">
                        <GitFork size={14} />
                        <span>新工作树</span>
                        <small>未实现</small>
                      </button>
                      <button disabled title="远程执行环境尚未实现">
                        <Cloud size={14} />
                        <span>云环境</span>
                        <small>未实现</small>
                      </button>
                    </div>
                  )}
                </>
              )}
            </div>
            <button
              className="composer-context-button"
              type="button"
              disabled
              title="分支读取尚未实现"
            >
              <GitBranch size={12} />
              <span>分支未接入</span>
            </button>
          </div>
        )}
        {(contextSources.length > 0 ||
          imageAttachments.length > 0 ||
          selectedSkillIds.length > 0) && (
          <div className="composer-sources" aria-label="已添加来源">
            {imageAttachments.map((attachment, index) => (
              <span className="composer-image-attachment" key={attachment.id}>
                <button
                  className="composer-image-preview-button"
                  type="button"
                  title={`预览 ${attachment.name}`}
                  aria-label={`预览 ${attachment.name}`}
                  onClick={() => setPreviewIndex(index)}
                >
                  <img src={attachment.previewUrl} alt={attachment.name} />
                </button>
                <IconButton
                  className="composer-image-remove"
                  size="compact"
                  type="button"
                  aria-label={`移除 ${attachment.name}`}
                  title={`移除 ${attachment.name}`}
                  onClick={() => removeImageAttachment(attachment.id)}
                >
                  <X size={14} aria-hidden="true" />
                </IconButton>
              </span>
            ))}
            {contextSources.map((source) => (
              <span
                className="composer-source"
                key={source.path}
                title={source.path}
              >
                <ContextSourceIcon extension={source.extension} />
                <span>{source.name}</span>
                <small>{formatBytes(source.bytes)}</small>
                <button
                  type="button"
                  title={`移除 ${source.name}`}
                  aria-label={`移除 ${source.name}`}
                  onClick={() => onRemoveContextSource(source.path)}
                >
                  <X size={12} />
                </button>
              </span>
            ))}
            {skills
              .filter((skill) => selectedSkillIds.includes(skill.id))
              .map((skill) => (
                <span
                  className="composer-source is-skill"
                  key={skill.id}
                  title={skill.description || skill.path}
                >
                  <Plug size={12} />
                  <span>{skill.name}</span>
                  <small>Skill</small>
                  <button
                    type="button"
                    title={`移除 ${skill.name}`}
                    aria-label={`移除 ${skill.name}`}
                    onClick={() => onToggleSkill(skill.id)}
                  >
                    <X size={12} />
                  </button>
                </span>
              ))}
          </div>
        )}
        {isDraggingFiles ? (
          <div className="composer-drop-target" aria-hidden="true">
            <Paperclip size={20} />
            <span>释放以添加文件</span>
          </div>
        ) : null}
        <textarea
          autoFocus={autoFocus}
          value={draft}
          aria-label="消息"
          placeholder={collaborationModePlaceholder(collaborationMode)}
          onFocus={closeMenus}
          onPointerDown={closeMenus}
          onPaste={(event) => void handlePaste(event)}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
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
        <div className="composer-toolbar">
          <div className="composer-menu-wrap">
            <button
              className="composer-icon-button"
              type="button"
              title="添加内容或选择模式"
              aria-label="添加内容或选择模式"
              aria-expanded={openMenu === "actions"}
              onClick={() =>
                setOpenMenu((current) =>
                  current === "actions" ? null : "actions",
                )
              }
            >
              <Plus size={16} />
            </button>
          </div>
          <div className="composer-menu-wrap">
            <button
              className="composer-mode"
              type="button"
              aria-expanded={openMenu === "permission"}
              onClick={() =>
                setOpenMenu((current) =>
                  current === "permission" ? null : "permission",
                )
              }
            >
              {permissionModeLabel(permissionMode)}
            </button>
            {openMenu === "permission" && (
              <div className="tool-popover permission-popover" role="menu">
                <div className="permission-popover-header">
                  <span>应如何批准 OpenTopia 操作？</span>
                  <span title="权限预设会同时调整审批策略和本地沙箱">
                    了解更多
                  </span>
                </div>
                {permissionModeOptions.map((option) => {
                  const Icon = option.icon;
                  const selected =
                    normalizedPermissionMode(permissionMode) === option.value;
                  return (
                    <button
                      className={`permission-option ${selected ? "active" : ""} ${option.value === "full_access" ? "is-danger" : ""}`}
                      disabled={isRunning || isSending}
                      key={option.value}
                      role="menuitemradio"
                      aria-checked={selected}
                      onClick={() => {
                        onChangePermissionMode(option.value);
                        setOpenMenu(null);
                      }}
                    >
                      <Icon size={17} aria-hidden="true" />
                      <span className="permission-option-copy">
                        <strong>{option.label}</strong>
                        <small>{option.detail}</small>
                      </span>
                      {selected ? <Check size={15} aria-hidden="true" /> : null}
                    </button>
                  );
                })}
              </div>
            )}
          </div>
          {queuedMessageCount > 0 ? (
            <span className="composer-queue-status">
              {queuedMessageCount} queued
            </span>
          ) : null}
          {/* Trails the toolbar so the model label sits beside the send button. */}
          <ModelSelector
            activeConnectionId={activeProviderId}
            connections={providers}
            disabled={isRunning || isSending}
            onChange={onChangeModelSelection}
            onOpenSettings={onOpenSettings}
            selection={modelSelection}
          />
        </div>
        <button
          className={`send-button${hasSendableContent ? " has-content" : ""}${isSending ? " is-sending" : ""}${isRunning ? " is-running" : ""}`}
          type="button"
          disabled={isRunning ? isCancelling : isSending || !hasSendableContent}
          onClick={isRunning ? onCancel : submitDraft}
          title={
            isRunning
              ? isCancelling
                ? "正在中断执行"
                : "中断执行"
              : isSending
                ? "正在发送消息"
                : "发送消息"
          }
          aria-label={
            isRunning
              ? isCancelling
                ? "正在中断智能体执行"
                : "中断智能体执行"
              : isSending
                ? "正在发送消息"
                : "发送消息"
          }
          aria-busy={isSending || isCancelling}
        >
          {isRunning ? (
            <Square
              className="stop-icon"
              size={15}
              fill="currentColor"
              aria-hidden="true"
            />
          ) : isSending ? (
            <Loader2 size={17} className="spin" aria-hidden="true" />
          ) : (
            <ArrowUp size={18} strokeWidth={2.25} aria-hidden="true" />
          )}
        </button>
        {openMenu === "actions" && (
          <div className="tool-popover composer-actions-popover" role="menu">
            <div className="composer-actions-section-label">添加</div>
            <button
              role="menuitem"
              onClick={() => {
                void onAddContextSources();
                setOpenMenu(null);
              }}
            >
              <Paperclip size={14} />
              <span>文件和文件夹</span>
            </button>
            <div className="tool-popover-separator" />
            <div className="composer-actions-section-label">模式</div>
            {collaborationModeOptions.map((option) => {
              const Icon = option.icon;
              const selected = collaborationMode === option.value;
              return (
                <button
                  className={`composer-mode-option is-${option.value} ${selected ? "active" : ""}`}
                  disabled={isRunning || isSending}
                  key={option.value}
                  role="menuitemcheckbox"
                  aria-checked={selected}
                  onClick={() => {
                    onChangeCollaborationMode(
                      selected ? "default" : option.value,
                    );
                    setOpenMenu(null);
                  }}
                >
                  <Icon size={15} aria-hidden="true" />
                  <span className="composer-action-copy">
                    <strong>{option.label}</strong>
                    <small>{option.detail}</small>
                  </span>
                  {selected ? <Check size={14} aria-hidden="true" /> : null}
                </button>
              );
            })}
            {skills.length > 0 ? (
              <>
                <div className="tool-popover-separator" />
                <div className="composer-actions-section-label">插件</div>
                {skills.map((skill) => {
                  const selected = selectedSkillIds.includes(skill.id);
                  return (
                    <button
                      className={`composer-tool-option ${selected ? "active" : ""}`}
                      key={skill.id}
                      role="menuitemcheckbox"
                      aria-checked={selected}
                      disabled={!selected && selectedSkillIds.length >= 5}
                      onClick={() => onToggleSkill(skill.id)}
                    >
                      <Plug size={14} aria-hidden="true" />
                      <span className="composer-action-copy">
                        <strong>{skill.name}</strong>
                        {skill.description ? (
                          <small>{skill.description}</small>
                        ) : null}
                      </span>
                      {selected ? <Check size={14} aria-hidden="true" /> : null}
                    </button>
                  );
                })}
              </>
            ) : null}
          </div>
        )}
      </div>
      {previewIndex !== null && imageAttachments[previewIndex] ? (
        <ImageLightbox
          attachments={imageAttachments}
          activeIndex={previewIndex}
          onChangeIndex={setPreviewIndex}
          onClose={() => setPreviewIndex(null)}
        />
      ) : null}
    </div>
  );
}

function ImageLightbox({
  attachments,
  activeIndex,
  onChangeIndex,
  onClose,
}: {
  attachments: ComposerImageAttachment[];
  activeIndex: number;
  onChangeIndex(index: number): void;
  onClose(): void;
}) {
  const [zoom, setZoom] = useState(1);
  const active = attachments[activeIndex];

  useEffect(() => {
    setZoom(1);
  }, [activeIndex]);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
      if (event.key === "ArrowLeft" && activeIndex > 0) {
        onChangeIndex(activeIndex - 1);
      }
      if (event.key === "ArrowRight" && activeIndex < attachments.length - 1) {
        onChangeIndex(activeIndex + 1);
      }
    };
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [activeIndex, attachments.length, onChangeIndex, onClose]);

  if (!active) return null;

  return createPortal(
    <div
      className="image-lightbox"
      role="dialog"
      aria-modal="true"
      aria-label={`预览 ${active.name}`}
      onPointerDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div className="image-lightbox-dialog">
        <header className="image-lightbox-header">
          <strong>{active.name}</strong>
          <span>
            {activeIndex + 1} / {attachments.length}
          </span>
          <IconButton aria-label="关闭图片预览" title="关闭" onClick={onClose}>
            <X size={18} aria-hidden="true" />
          </IconButton>
        </header>
        <div className="image-lightbox-stage">
          <IconButton
            aria-label="上一张图片"
            title="上一张"
            disabled={activeIndex === 0}
            onClick={() => onChangeIndex(activeIndex - 1)}
          >
            <ChevronLeft size={20} aria-hidden="true" />
          </IconButton>
          <div className="image-lightbox-canvas">
            <img
              src={active.previewUrl}
              alt={active.name}
              draggable={false}
              style={{ transform: `scale(${zoom})` }}
            />
          </div>
          <IconButton
            aria-label="下一张图片"
            title="下一张"
            disabled={activeIndex === attachments.length - 1}
            onClick={() => onChangeIndex(activeIndex + 1)}
          >
            <ChevronRight size={20} aria-hidden="true" />
          </IconButton>
        </div>
        <footer className="image-lightbox-footer">
          <button
            className="image-lightbox-reset"
            type="button"
            onClick={() => setZoom(1)}
            disabled={zoom === 1}
          >
            <RotateCcw size={16} aria-hidden="true" />
            <span>重置</span>
          </button>
          <div className="image-lightbox-zoom-controls">
            <IconButton
              aria-label="缩小图片"
              title="缩小"
              disabled={zoom <= 0.5}
              onClick={() => setZoom((current) => Math.max(0.5, current - 0.5))}
            >
              <ZoomOut size={16} aria-hidden="true" />
            </IconButton>
            <span>{Math.round(zoom * 100)}%</span>
            <IconButton
              aria-label="放大图片"
              title="放大"
              disabled={zoom >= 3}
              onClick={() => setZoom((current) => Math.min(3, current + 0.5))}
            >
              <ZoomIn size={16} aria-hidden="true" />
            </IconButton>
          </div>
          <a
            className="image-lightbox-download"
            href={active.previewUrl}
            download={active.name}
          >
            <Download size={15} aria-hidden="true" />
            <span>下载</span>
          </a>
        </footer>
      </div>
    </div>,
    document.body,
  );
}

const collaborationModeOptions: Array<{
  value: CollaborationMode;
  label: string;
  detail: string;
  icon: typeof Zap;
}> = [
  {
    value: "goal",
    label: "目标",
    detail: "设置要持续追求的目标",
    icon: Target,
  },
  {
    value: "plan",
    label: "计划模式",
    detail: "开启计划模式",
    icon: ListTodo,
  },
];

function collaborationModePlaceholder(mode: CollaborationMode): string {
  if (mode === "goal") return "描述要持续推进的目标";
  if (mode === "plan") return "描述需要调研和规划的任务";
  return "请求后续更改";
}

const permissionModeOptions: Array<{
  value: ExecutionPermissionMode;
  label: string;
  detail: string;
  icon: typeof Hand;
}> = [
  {
    value: "approve",
    label: "请求批准",
    detail: "编辑外部文件和使用互联网时始终询问",
    icon: Hand,
  },
  {
    value: "auto",
    label: "替我审批",
    detail: "仅对检测到的风险操作请求批准",
    icon: ShieldCheck,
  },
  {
    value: "full_access",
    label: "完全访问权限",
    detail: "可不受限制地访问互联网和此电脑上的任何文件",
    icon: ShieldAlert,
  },
];

const sandboxModeOptions: Array<{
  value: AppSettings["sandbox"]["sandboxMode"];
  label: string;
  detail: string;
}> = [
  { value: "read-only", label: "只读沙箱", detail: "禁止写入" },
  { value: "workspace-write", label: "工作区写入", detail: "默认" },
  { value: "danger-full-access", label: "完全访问", detail: "无 OS 沙箱" },
];

function sandboxModeLabel(mode: AppSettings["sandbox"]["sandboxMode"]): string {
  return (
    sandboxModeOptions.find((option) => option.value === mode)?.label ?? mode
  );
}

function permissionModeLabel(mode: AppSettings["permissionMode"]): string {
  switch (normalizedPermissionMode(mode)) {
    case "full_access":
      return "完全访问权限";
    case "approve":
      return "请求批准";
    default:
      return "替我审批";
  }
}

function normalizedPermissionMode(
  mode: AppSettings["permissionMode"],
): ExecutionPermissionMode {
  return mode === "approve" || mode === "full_access" ? mode : "auto";
}

function SideTaskConversation({
  client,
  thread,
  settings,
  projects,
  skills,
  initialCollaborationMode,
  onThreadUpdated,
  onSetThreadActivity,
  onChangePermissionMode,
  onChangeSandboxMode,
  onOpenSettings,
  onOpenArtifact,
  onOpenMarkdownLink,
  onOpenToolTab,
  onOpenFileReview,
}: {
  client: ApiClient | null;
  thread: Thread | null;
  settings: AppSettings | null;
  projects: Project[];
  skills: SkillDescriptor[];
  initialCollaborationMode: CollaborationMode;
  onThreadUpdated(thread: Thread): void;
  onSetThreadActivity(
    threadId: string,
    status: ThreadActivityStatus | null,
  ): void;
  onChangePermissionMode(mode: ExecutionPermissionMode): void;
  onChangeSandboxMode(mode: AppSettings["sandbox"]["sandboxMode"]): void;
  onOpenSettings(): void;
  onOpenArtifact(threadId: string, artifactId: string): void;
  onOpenMarkdownLink(href: string, baseWorkspacePath?: string | null): void;
  onOpenToolTab(kind: ToolTabKind): void;
  onOpenFileReview(path: string): void;
}) {
  const threadId = thread?.id ?? null;
  const [messages, setMessages] = useState<Message[]>([]);
  const [events, setEvents] = useState<AgentEvent[]>([]);
  const [loadState, setLoadState] = useState<ConversationLoadState>({
    threadId,
    status: threadId ? "loading" : "idle",
    error: null,
  });
  const [loadAttempt, setLoadAttempt] = useState(0);
  const [composer, setComposer] = useState("");
  const [isSending, setIsSending] = useState(false);
  const [activeTurnId, setActiveTurnId] = useState<string | null>(null);
  const [cancellingTurnId, setCancellingTurnId] = useState<string | null>(null);
  const [pendingTurnFeedback, setPendingTurnFeedback] =
    useState<PendingTurnFeedback | null>(null);
  const [queuedMessageCount, setQueuedMessageCount] = useState(0);
  const [contextSources, setContextSources] = useState<ContextSourceFile[]>([]);
  const [selectedSkillIds, setSelectedSkillIds] = useState<string[]>([]);
  const [collaborationMode, setCollaborationMode] = useState(
    initialCollaborationMode,
  );
  const [modelSelection, setModelSelection] =
    useState<ThreadModelSelection | null>(thread?.modelSelection ?? null);
  const [pendingApprovalIds, setPendingApprovalIds] = useState<string[]>([]);
  const [decidingApprovalId, setDecidingApprovalId] = useState<string | null>(
    null,
  );
  const [approvalError, setApprovalError] = useState<string | null>(null);
  const [pendingUserInput, setPendingUserInput] = useState<UserInputRecord[]>(
    [],
  );
  const [submittingUserInputId, setSubmittingUserInputId] = useState<
    string | null
  >(null);
  const [userInputError, setUserInputError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [undoingTurnId, setUndoingTurnId] = useState<string | null>(null);
  const eventIdsRef = useRef(new Set<string>());

  useEffect(() => {
    setModelSelection(thread?.modelSelection ?? null);
  }, [thread?.modelSelection]);

  const ingestSideTaskEvent = useCallback(
    (event: AgentEvent) => {
      if (!threadId || event.threadId !== threadId) return;
      if (eventIdsRef.current.has(event.id)) return;
      eventIdsRef.current.add(event.id);
      setEvents((current) =>
        [...current, event].sort((left, right) => left.seq - right.seq),
      );

      if (event.payload.type === "assistant_message") {
        const assistantMessage = event.payload.message;
        setMessages((current) =>
          current.some((message) => message.id === assistantMessage.id)
            ? current
            : [...current, assistantMessage],
        );
      }
      if (event.payload.type === "approval_requested") {
        const approvalId = event.payload.approval_id;
        setPendingApprovalIds((current) =>
          current.includes(approvalId) ? current : [...current, approvalId],
        );
        onSetThreadActivity(threadId, "approval");
      }
      if (event.payload.type === "browser_handoff_required") {
        onSetThreadActivity(threadId, "user_action");
      }
      if (event.payload.type === "user_input_requested") {
        const request = event.payload.request;
        setPendingUserInput((current) =>
          current.some(
            (record) => record.request.requestId === request.requestId,
          )
            ? current
            : [
                ...current,
                {
                  threadId,
                  request,
                  status: "pending",
                  response: null,
                  createdAt: event.createdAt,
                  answeredAt: null,
                },
              ],
        );
      }

      if (event.payload.type === "turn_started" && event.turnId) {
        setActiveTurnId(event.turnId);
        setCancellingTurnId(null);
        setQueuedMessageCount((current) => Math.max(0, current - 1));
        onSetThreadActivity(threadId, "processing");
      } else if (event.payload.type === "turn_finished") {
        setActiveTurnId((current) =>
          !event.turnId || current === event.turnId ? null : current,
        );
        setCancellingTurnId(null);
        setPendingTurnFeedback(null);
        onSetThreadActivity(threadId, "succeeded");
      } else if (event.payload.type === "turn_suspended") {
        setActiveTurnId((current) =>
          !event.turnId || current === event.turnId ? null : current,
        );
        setCancellingTurnId(null);
        onSetThreadActivity(threadId, "approval");
      } else if (event.payload.type === "browser_handoff_required") {
        setActiveTurnId((current) =>
          !event.turnId || current === event.turnId ? null : current,
        );
        setCancellingTurnId(null);
        setPendingTurnFeedback(null);
      } else if (event.payload.type === "turn_cancelled") {
        setActiveTurnId((current) =>
          !event.turnId || current === event.turnId ? null : current,
        );
        setCancellingTurnId(null);
        setPendingTurnFeedback(null);
        onSetThreadActivity(threadId, null);
      } else if (
        event.payload.type === "turn_awaiting_input" ||
        event.payload.type === "error"
      ) {
        setActiveTurnId((current) =>
          !event.turnId || current === event.turnId ? null : current,
        );
        setCancellingTurnId(null);
      }

      if (event.payload.type === "error") {
        setPendingTurnFeedback(null);
        setActionError(friendlyProviderError(event.payload.message));
        onSetThreadActivity(threadId, "failed");
      }
    },
    [onSetThreadActivity, threadId],
  );

  useEffect(() => {
    if (!client || !threadId) {
      setLoadState({ threadId: null, status: "idle", error: null });
      return;
    }

    let cancelled = false;
    let source: StreamHandle | null = null;
    eventIdsRef.current = new Set();
    setMessages([]);
    setEvents([]);
    setPendingApprovalIds([]);
    setPendingUserInput([]);
    setLoadState({ threadId, status: "loading", error: null });

    void Promise.all([
      client.listMessages(threadId),
      client.listEvents(threadId),
      client.getTurnStatus(threadId),
      client.listPendingApprovals(threadId),
      client.listPendingUserInput(threadId),
    ])
      .then(
        ([
          loadedMessages,
          loadedEvents,
          turnStatus,
          pendingApprovals,
          pendingPlanningInput,
        ]) => {
          if (cancelled) return;
          loadedEvents.forEach((event) => eventIdsRef.current.add(event.id));
          setMessages(loadedMessages);
          setEvents(loadedEvents);
          setActiveTurnId(
            turnStatus?.status === "running" ||
              turnStatus?.status === "cancelling"
              ? turnStatus.turnId
              : null,
          );
          setPendingApprovalIds(
            pendingApprovals.map((approval) => approval.approvalId),
          );
          setPendingUserInput(pendingPlanningInput);
          onSetThreadActivity(
            threadId,
            pendingApprovals.length > 0
              ? "approval"
              : resolveThreadActivityStatus(turnStatus),
          );
          setLoadState({ threadId, status: "ready", error: null });
          source = client.openEventStream(
            threadId,
            loadedEvents.at(-1)?.seq,
            ingestSideTaskEvent,
          );
        },
      )
      .catch((error) => {
        if (cancelled) return;
        setLoadState({
          threadId,
          status: "error",
          error: errorMessage(error),
        });
      });

    return () => {
      cancelled = true;
      source?.close();
    };
  }, [client, ingestSideTaskEvent, loadAttempt, onSetThreadActivity, threadId]);

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
  const taskPlan = useMemo(
    () => resolveComposerTaskPlan(events, null),
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
  ): Promise<boolean> {
    const messageText = input.trim();
    if (
      !client ||
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
    const startedAt = new Date().toISOString();
    setIsSending(true);
    setActionError(null);
    setPendingTurnFeedback({
      threadId: thread.id,
      turnId: null,
      phase: "thinking",
      startedAt,
    });
    try {
      const { message, turnId, queued } = await client.sendMessage(
        thread.id,
        messageText,
        contextSources.map((source) => source.path),
        selectedSkillIds,
        collaborationMode,
        undefined,
        imageAttachments,
      );
      setMessages((current) => [...current, message]);
      setActiveTurnId(turnId);
      setPendingTurnFeedback((current) =>
        current?.startedAt === startedAt
          ? {
              ...current,
              turnId,
              phase: queued ? "processing" : current.phase,
            }
          : current,
      );
      if (queued) setQueuedMessageCount((current) => current + 1);
      setComposer("");
      setContextSources([]);
      setSelectedSkillIds([]);
      onSetThreadActivity(thread.id, "processing");
      if (isFirstPrompt && messageText) void updateThreadTitle(messageText);
      return true;
    } catch (error) {
      setPendingTurnFeedback((current) =>
        current?.startedAt === startedAt ? null : current,
      );
      setActionError(errorMessage(error));
      onSetThreadActivity(thread.id, "failed");
      return false;
    } finally {
      setIsSending(false);
    }
  }

  async function cancelSideTaskTurn() {
    if (!client || !thread || !activeTurnId || cancellingTurnId) return;
    setCancellingTurnId(activeTurnId);
    setActionError(null);
    try {
      const result = await client.cancelTurn(thread.id, activeTurnId);
      if (!result.cancelled) throw new Error(result.message);
    } catch (error) {
      setCancellingTurnId(null);
      setActionError(errorMessage(error));
    }
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
    if (!client || !thread || decidingApprovalId) return;
    setDecidingApprovalId(approvalId);
    setApprovalError(null);
    try {
      const decision = await client.decideApproval(
        thread.id,
        approvalId,
        approved,
      );
      if (!decision.accepted) throw new Error("服务端未接受该审批决定。");
      setPendingApprovalIds((current) =>
        current.filter((id) => id !== approvalId),
      );
    } catch (error) {
      setApprovalError(`审批决定提交失败：${errorMessage(error)}`);
    } finally {
      setDecidingApprovalId(null);
    }
  }

  async function submitSideTaskUserInput(
    requestId: string,
    response: UserInputResponse,
  ) {
    if (!client || !thread || submittingUserInputId) return;
    setSubmittingUserInputId(requestId);
    setUserInputError(null);
    try {
      const result = await client.respondToUserInput(
        thread.id,
        requestId,
        response,
      );
      if (!result.accepted || !result.resumed) {
        throw new Error("服务端未恢复侧边任务。");
      }
      setPendingUserInput((current) =>
        current.filter((record) => record.request.requestId !== requestId),
      );
    } catch (error) {
      setUserInputError(`无法提交选择：${errorMessage(error)}`);
    } finally {
      setSubmittingUserInputId(null);
    }
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
    <section className="side-task-conversation" aria-label="侧边任务会话">
      {loadState.status === "error" ? (
        <ConversationLoadErrorState
          error={loadState.error ?? "无法加载侧边任务"}
          onRetry={() => setLoadAttempt((current) => current + 1)}
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
      {actionError ? (
        <div className="side-task-conversation-error" role="alert">
          <AlertCircle size={14} aria-hidden="true" />
          <span>{actionError}</span>
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
        />
      ) : (
        <Composer
          autoFocus
          value={composer}
          taskPlan={taskPlan}
          isSending={isSending}
          isRunning={Boolean(activeTurnId)}
          isCancelling={
            Boolean(activeTurnId) && cancellingTurnId === activeTurnId
          }
          queuedMessageCount={queuedMessageCount}
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

function RightPanel({
  client,
  threads,
  toolTabs,
  activeToolTab,
  toolStageOpen,
  conversationCollapsed,
  contextRailOpen,
  contextRailAutoVisible,
  thread,
  settings,
  projects,
  skills,
  collaborationMode,
  workspaceRoot,
  subagentRuns,
  messages,
  events,
  terminalEvents,
  terminalSession,
  workspaceTree,
  filePreview,
  workspaceDiff,
  sandbox,
  plugins,
  selectedSkillIds,
  mcpServers,
  threadMcpServers,
  workbenchError,
  isRefreshingWorkbench,
  pendingApprovalIds,
  decidingApprovalId,
  artifacts,
  contextStatus,
  isCompactingContext,
  revertingDiffPath,
  hunkActionKey,
  reviewFileRequest,
  onDecideApproval,
  onRefreshWorkbench,
  onOpenWorkspacePath,
  onOpenWorkspaceEntry,
  onToggleThreadMcp,
  onCreateMcpServer,
  onUpdateMcpServer,
  onRestartMcpServer,
  onDeleteMcpServer,
  onInstallPlugin,
  onUninstallPlugin,
  onToggleThreadPlugin,
  onUsePluginSkills,
  onOpenWorkspace,
  onEnsureTerminalSession,
  onWriteTerminalSession,
  onResizeTerminalSession,
  onCloseTerminalSession,
  onCompactContext,
  onOpenArtifact,
  onOpenMarkdownLink,
  onRevertDiffFile,
  onApplyDiffHunk,
  onOpenFileTab,
  onLoadFileContent,
  onLoadTurnFileDiff,
  onGitAction,
  onGetArtifact,
  onOpenToolTab,
  onOpenSideTask,
  onThreadUpdated,
  onSetThreadActivity,
  onChangePermissionMode,
  onChangeSandboxMode,
  onOpenSettings,
  onActivateToolTab,
  onCloseToolTab,
  onToggleConversation,
  onHideToolStage,
  onAddContextSources,
  onCancelSubagent,
}: {
  client: ApiClient | null;
  threads: Thread[];
  toolTabs: ToolTab[];
  activeToolTab: ToolTab | null;
  toolStageOpen: boolean;
  conversationCollapsed: boolean;
  contextRailOpen: boolean;
  contextRailAutoVisible: boolean;
  thread: Thread | null;
  settings: AppSettings | null;
  projects: Project[];
  skills: SkillDescriptor[];
  collaborationMode: CollaborationMode;
  workspaceRoot: string | null;
  messages: Message[];
  subagentRuns: SubagentRun[];
  events: AgentEvent[];
  terminalEvents: TerminalEvent[];
  terminalSession: TerminalSession | null;
  workspaceTree: WorkspaceTree | null;
  filePreview: WorkspaceFilePreview | null;
  workspaceDiff: WorkspaceDiff | null;
  sandbox: SandboxDescriptor | null;
  plugins: PluginView[];
  selectedSkillIds: string[];
  mcpServers: McpServerView[];
  threadMcpServers: ThreadMcpServerView[];
  workbenchError: string | null;
  isRefreshingWorkbench: boolean;
  pendingApprovalIds: string[];
  decidingApprovalId: string | null;
  artifacts: ArtifactDescriptor[];
  contextStatus: ContextStatus | null;
  isCompactingContext: boolean;
  revertingDiffPath: string | null;
  hunkActionKey: string | null;
  reviewFileRequest: ReviewFileRequest | null;
  onDecideApproval(approvalId: string, approved: boolean): void;
  onRefreshWorkbench(): void;
  onOpenWorkspacePath(path?: string): void;
  onOpenWorkspaceEntry(entry: WorkspaceEntry): void;
  onToggleThreadMcp(serverId: string, enabled: boolean): void;
  onCreateMcpServer(input: McpServerInput): Promise<void>;
  onUpdateMcpServer(serverId: string, input: McpServerInput): Promise<void>;
  onRestartMcpServer(serverId: string): Promise<void>;
  onDeleteMcpServer(serverId: string): Promise<void>;
  onInstallPlugin(): Promise<void>;
  onUninstallPlugin(pluginId: string): Promise<void>;
  onToggleThreadPlugin(pluginId: string, enabled: boolean): Promise<void>;
  onUsePluginSkills(pluginId: string, enabled: boolean): void;
  onOpenWorkspace(workspaceRoot: string): void;
  onEnsureTerminalSession(threadId: string): Promise<TerminalSession>;
  onWriteTerminalSession(
    threadId: string,
    sessionId: string,
    data: string,
  ): void;
  onResizeTerminalSession(
    threadId: string,
    sessionId: string,
    cols: number,
    rows: number,
  ): void;
  onCloseTerminalSession(threadId: string, sessionId: string): void;
  onCompactContext(): void;
  onOpenArtifact(threadId: string, artifactId: string): void;
  onOpenMarkdownLink(href: string, baseWorkspacePath?: string | null): void;
  onRevertDiffFile(path: string): void;
  onApplyDiffHunk(
    hunk: WorkspaceDiffHunk,
    action: WorkspaceDiffHunkAction,
  ): void;
  onOpenFileTab(path: string): void;
  onLoadFileContent(path: string): Promise<DiffReviewFileContent>;
  onLoadTurnFileDiff(turnId: string, path: string): Promise<string>;
  onGitAction(action: DiffReviewGitAction, message: string): Promise<string>;
  onGetArtifact(threadId: string, artifactId: string): Promise<ArtifactContent>;
  onOpenToolTab(kind: ToolTabKind): void;
  onOpenSideTask(): void;
  onThreadUpdated(thread: Thread): void;
  onSetThreadActivity(
    threadId: string,
    status: ThreadActivityStatus | null,
  ): void;
  onChangePermissionMode(mode: ExecutionPermissionMode): void;
  onChangeSandboxMode(mode: AppSettings["sandbox"]["sandboxMode"]): void;
  onOpenSettings(): void;
  onActivateToolTab(tabId: string): void;
  onCloseToolTab(tabId: string): void;
  onToggleConversation(): void;
  onHideToolStage(): void;
  onAddContextSources(): void;
  onCancelSubagent(runId: string): void;
}) {
  const renderWorkbench = (
    mode: "panel" | "stage",
    activeTab?: WorkbenchTab,
  ) => (
    <WorkbenchPanel
      client={client}
      mode={mode}
      activeTab={activeTab}
      thread={thread}
      workspaceRoot={workspaceRoot}
      events={events}
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
      decidingApprovalId={decidingApprovalId}
      artifacts={artifacts}
      contextStatus={contextStatus}
      isCompactingContext={isCompactingContext}
      revertingDiffPath={revertingDiffPath}
      hunkActionKey={hunkActionKey}
      reviewFileRequest={reviewFileRequest}
      onDecideApproval={onDecideApproval}
      onRefreshWorkbench={onRefreshWorkbench}
      onOpenWorkspacePath={onOpenWorkspacePath}
      onOpenWorkspaceEntry={onOpenWorkspaceEntry}
      onToggleThreadMcp={onToggleThreadMcp}
      onCreateMcpServer={onCreateMcpServer}
      onUpdateMcpServer={onUpdateMcpServer}
      onRestartMcpServer={onRestartMcpServer}
      onDeleteMcpServer={onDeleteMcpServer}
      onInstallPlugin={onInstallPlugin}
      onUninstallPlugin={onUninstallPlugin}
      onToggleThreadPlugin={onToggleThreadPlugin}
      onUsePluginSkills={onUsePluginSkills}
      onOpenPath={onOpenWorkspace}
      onEnsureTerminalSession={onEnsureTerminalSession}
      onWriteTerminalSession={onWriteTerminalSession}
      onResizeTerminalSession={onResizeTerminalSession}
      onCloseTerminalSession={onCloseTerminalSession}
      onCompactContext={onCompactContext}
      onOpenArtifact={onOpenArtifact}
      onRevertDiffFile={onRevertDiffFile}
      onApplyDiffHunk={onApplyDiffHunk}
      onOpenFileTab={onOpenFileTab}
      onLoadFileContent={onLoadFileContent}
      onLoadTurnFileDiff={onLoadTurnFileDiff}
      onGitAction={onGitAction}
      onGetArtifact={onGetArtifact}
    />
  );

  if (toolStageOpen) {
    return (
      <aside className="right-panel tool-stage" id="workspace-right-panel">
        <ToolTabStrip
          tabs={toolTabs}
          activeTabId={activeToolTab?.id ?? null}
          onActivate={onActivateToolTab}
          onClose={onCloseToolTab}
          onOpen={onOpenToolTab}
          onOpenSideTask={onOpenSideTask}
          canOpenSideTask={Boolean(thread)}
          conversationCollapsed={conversationCollapsed}
          onToggleConversation={onToggleConversation}
          onHide={onHideToolStage}
        />
        <div className="tool-stage-body">
          {!activeToolTab ? (
            <ToolStageLauncher onOpen={onOpenToolTab} />
          ) : activeToolTab.kind === "side-task" ? (
            activeToolTab.sideTaskThreadId ? (
              <SideTaskConversation
                key={activeToolTab.sideTaskThreadId}
                client={client}
                thread={
                  threads.find(
                    (item) => item.id === activeToolTab.sideTaskThreadId,
                  ) ?? null
                }
                settings={settings}
                projects={projects}
                skills={skills}
                initialCollaborationMode={collaborationMode}
                onThreadUpdated={onThreadUpdated}
                onSetThreadActivity={onSetThreadActivity}
                onChangePermissionMode={onChangePermissionMode}
                onChangeSandboxMode={onChangeSandboxMode}
                onOpenSettings={onOpenSettings}
                onOpenArtifact={onOpenArtifact}
                onOpenMarkdownLink={onOpenMarkdownLink}
                onOpenToolTab={onOpenToolTab}
                onOpenFileReview={onOpenFileTab}
              />
            ) : (
              <ConversationLoadingState />
            )
          ) : activeToolTab.kind === "browser" ? (
            <WebPreviewSurface
              client={client}
              threadId={thread?.id ?? null}
              events={events}
              navigationRequest={activeToolTab.browserNavigation ?? null}
            />
          ) : activeToolTab.kind === "computer" ? (
            <ComputerPanel client={client} threadId={thread?.id ?? null} />
          ) : activeToolTab.kind === "preview" &&
            activeToolTab.previewTarget ? (
            <PreviewHost
              client={client}
              threadId={thread?.id ?? null}
              workspaceRoot={workspaceRoot}
              target={activeToolTab.previewTarget}
              onOpenMarkdownLink={onOpenMarkdownLink}
            />
          ) : (
            activeToolTab.kind !== "preview" &&
            renderWorkbench("stage", activeToolTab.kind)
          )}
        </div>
      </aside>
    );
  }

  return (
    <aside
      className={`context-rail-shell ${
        contextRailOpen ? "is-visible" : ""
      } ${contextRailOpen ? (contextRailAutoVisible ? "is-inline" : "is-menu") : ""}`}
      id="workspace-context-rail"
      aria-label="右侧上下文摘要"
      aria-hidden={!contextRailOpen}
    >
      <RightContextRail
        client={client}
        threadId={thread?.id ?? null}
        workspaceRoot={workspaceRoot}
        workspaceDiff={workspaceDiff}
        terminalEvents={terminalEvents}
        terminalSession={terminalSession}
        agentEvents={events}
        subagentRuns={subagentRuns}
        artifacts={artifacts}
        messages={messages}
        onOpenDiff={() => onOpenToolTab("diff")}
        onOpenTerminal={() => onOpenToolTab("terminal")}
        onOpenFiles={() => onOpenToolTab("files")}
        onOpenEnvironment={() => onOpenToolTab("sandbox")}
        onAddSource={onAddContextSources}
        onCancelSubagent={onCancelSubagent}
        onGitChanged={onRefreshWorkbench}
      />
    </aside>
  );
}

function ToolStageLauncher({
  onOpen,
}: {
  onOpen(kind: Exclude<ToolTabKind, "preview" | "side-task">): void;
}) {
  return (
    <div className="tool-stage-empty">
      <nav className="tool-stage-launcher" aria-label="打开工具">
        {toolStageLauncherKinds.map(({ kind, label }) => {
          const Icon = toolTabIcon(kind);
          return (
            <Button
              className="tool-stage-launcher-button"
              key={kind}
              variant="quiet"
              onClick={() => onOpen(kind)}
            >
              <Icon size={16} aria-hidden="true" />
              <span>{label}</span>
            </Button>
          );
        })}
      </nav>
    </div>
  );
}

function ToolTabStrip({
  tabs,
  activeTabId,
  onActivate,
  onClose,
  onOpen,
  onOpenSideTask,
  canOpenSideTask,
  conversationCollapsed,
  onToggleConversation,
  onHide,
}: {
  tabs: ToolTab[];
  activeTabId: string | null;
  onActivate(tabId: string): void;
  onClose(tabId: string): void;
  onOpen(kind: ToolTabKind): void;
  onOpenSideTask(): void;
  canOpenSideTask: boolean;
  conversationCollapsed: boolean;
  onToggleConversation(): void;
  onHide(): void;
}) {
  function open(kind: ToolTabKind, close: () => void) {
    onOpen(kind);
    close();
  }

  return (
    <div className="tool-tab-strip">
      <div className="tool-tab-list" role="tablist" aria-label="工作工具">
        {tabs.map((tab) => {
          const Icon = toolTabIcon(tab.kind);
          return (
            <div
              className={`tool-stage-tab ${tab.id === activeTabId ? "active" : ""}`}
              key={tab.id}
            >
              <button
                className="tool-tab-main"
                type="button"
                role="tab"
                aria-selected={tab.id === activeTabId}
                onClick={() => onActivate(tab.id)}
              >
                <Icon size={13} />
                <span>{tab.title}</span>
              </button>
              <button
                className="tool-tab-close"
                type="button"
                aria-label={`关闭 ${tab.title}`}
                onClick={(event) => {
                  event.stopPropagation();
                  onClose(tab.id);
                }}
              >
                <X size={12} />
              </button>
            </div>
          );
        })}
      </div>
      <div className="tool-tab-add-wrap">
        <Popover
          label="打开工具"
          align="end"
          placement="bottom"
          trigger={(props) => (
            <IconButton
              className="tool-tab-add"
              size="compact"
              variant="quiet"
              title="打开工具"
              aria-label="打开工具"
              {...props}
            >
              <Plus size={14} aria-hidden="true" />
            </IconButton>
          )}
        >
          {({ close }) => (
            <div className="tool-popover tool-tab-add-popover" role="menu">
              {toolTabMenuItems.map(({ kind, shortcut }) => {
                const Icon = toolTabIcon(kind);
                return (
                  <button
                    key={kind}
                    role="menuitem"
                    onClick={() => open(kind, close)}
                  >
                    <Icon size={14} aria-hidden="true" />
                    <span>{toolTabTitle(kind)}</span>
                    {shortcut ? <kbd>{shortcut}</kbd> : null}
                  </button>
                );
              })}
              <button
                type="button"
                role="menuitem"
                disabled={!canOpenSideTask}
                title={canOpenSideTask ? "新建侧边会话" : "请先打开一个任务"}
                onClick={() => {
                  close();
                  onOpenSideTask();
                }}
              >
                <CirclePlus size={14} aria-hidden="true" />
                <span>侧边任务</span>
                <kbd>Ctrl+Alt+S</kbd>
              </button>
            </div>
          )}
        </Popover>
      </div>
      <div className="tool-tab-actions">
        <IconButton
          className="tool-tab-action"
          size="compact"
          variant="quiet"
          title={conversationCollapsed ? "还原工具工作区" : "扩展工具工作区"}
          aria-label={
            conversationCollapsed ? "还原工具工作区" : "扩展工具工作区"
          }
          aria-pressed={conversationCollapsed}
          onClick={onToggleConversation}
        >
          {conversationCollapsed ? (
            <Minimize2 size={14} aria-hidden="true" />
          ) : (
            <Maximize2 size={14} aria-hidden="true" />
          )}
        </IconButton>
        <IconButton
          className="tool-tab-action"
          size="compact"
          variant="quiet"
          title="折叠工具窗口"
          aria-label="折叠工具窗口"
          onClick={onHide}
        >
          <PanelRightClose size={14} aria-hidden="true" />
        </IconButton>
      </div>
    </div>
  );
}

function NewTaskState({
  value,
  workspaceRoot,
  projectName,
  projects,
  modelSelection,
  providers,
  activeProviderId,
  permissionMode,
  collaborationMode,
  sandboxMode,
  contextSources,
  skills,
  selectedSkillIds,
  isSending,
  launchMode,
  experienceMode,
  onChange,
  onChangeLaunchMode,
  onPickWorkspace,
  onSelectProject,
  onChangePermissionMode,
  onChangeCollaborationMode,
  onChangeSandboxMode,
  onChangeModelSelection,
  onOpenSettings,
  onAddContextSources,
  onRemoveContextSource,
  onToggleSkill,
  onSubmit,
}: {
  value: string;
  workspaceRoot: string | null;
  projectName: string | null;
  projects: Project[];
  modelSelection: ThreadModelSelection | null;
  providers: ProviderSettings[];
  activeProviderId: string;
  permissionMode: AppSettings["permissionMode"];
  collaborationMode: CollaborationMode;
  sandboxMode: AppSettings["sandbox"]["sandboxMode"];
  contextSources: ContextSourceFile[];
  skills: SkillDescriptor[];
  selectedSkillIds: string[];
  isSending: boolean;
  launchMode: NewTaskLaunchMode;
  experienceMode: ExperienceMode;
  onChange(value: string): void;
  onChangeLaunchMode(mode: NewTaskLaunchMode): void;
  onPickWorkspace(): void;
  onSelectProject(projectId: string): void;
  onChangePermissionMode(mode: ExecutionPermissionMode): void;
  onChangeCollaborationMode(mode: CollaborationMode): void;
  onChangeSandboxMode(mode: AppSettings["sandbox"]["sandboxMode"]): void;
  onChangeModelSelection(selection: ThreadModelSelection): void;
  onOpenSettings(): void;
  onAddContextSources(files?: File[]): Promise<void>;
  onRemoveContextSource(path: string): void;
  onToggleSkill(skillId: string): void;
  onSubmit(
    value: string,
    imageAttachments: InlineImageAttachment[],
  ): Promise<boolean>;
}) {
  const suggestions =
    experienceMode === "work"
      ? [
          {
            icon: Search,
            label: "研究并汇总资料",
            prompt: "研究这个主题，核对来源并整理成清晰的结论",
          },
          {
            icon: FileText,
            label: "撰写与整理文档",
            prompt: "根据项目资料撰写并整理一份完整文档",
          },
          {
            icon: Table2,
            label: "分析表格与数据",
            prompt: "分析项目中的表格和数据，并总结关键发现",
          },
          {
            icon: Presentation,
            label: "制作演示或报告",
            prompt: "根据项目内容制作一份结构清晰的演示或报告",
          },
        ]
      : [
          {
            icon: Search,
            label: "探索并理解代码",
            prompt: "分析这个项目的架构和核心模块",
          },
          {
            icon: FileCode2,
            label: "构建新功能",
            prompt: "为这个项目实现一个新功能",
          },
          {
            icon: Check,
            label: "审查代码更改",
            prompt: "审查当前工作区中的代码更改",
          },
          {
            icon: Activity,
            label: "修复问题",
            prompt: "检查并修复当前项目中的问题",
          },
        ];

  return (
    <>
      <div className="new-task-state">
        <Bot size={34} />
        <h2>
          {experienceMode === "work" ? "今天想在" : "我们应该在"}{" "}
          <u>
            {projectName ??
              (workspaceRoot ? workspaceName(workspaceRoot) : "项目")}
          </u>{" "}
          {experienceMode === "work" ? "中完成什么？" : "中构建什么？"}
        </h2>
        <div className="task-suggestions">
          {suggestions.map((suggestion) => {
            const Icon = suggestion.icon;
            return (
              <button
                key={suggestion.label}
                type="button"
                onClick={() => onChange(suggestion.prompt)}
              >
                <Icon size={15} />
                <span>{suggestion.label}</span>
              </button>
            );
          })}
        </div>
        {!workspaceRoot && (
          <button className="workspace-picker-button" onClick={onPickWorkspace}>
            <Folder size={15} />
            选择项目文件夹
          </button>
        )}
      </div>
      <Composer
        value={value}
        isSending={isSending}
        isRunning={false}
        isCancelling={false}
        modelSelection={modelSelection}
        providers={providers}
        activeProviderId={activeProviderId}
        permissionMode={permissionMode}
        collaborationMode={collaborationMode}
        sandboxMode={sandboxMode}
        contextSources={contextSources}
        skills={skills}
        selectedSkillIds={selectedSkillIds}
        launchMode={launchMode}
        workspaceRoot={workspaceRoot}
        projectName={
          projectName ?? (workspaceRoot ? workspaceName(workspaceRoot) : null)
        }
        projects={projects}
        onChange={onChange}
        onSubmit={onSubmit}
        onCancel={() => undefined}
        onPickWorkspace={onPickWorkspace}
        onSelectProject={onSelectProject}
        onChangeLaunchMode={onChangeLaunchMode}
        onChangePermissionMode={onChangePermissionMode}
        onChangeCollaborationMode={onChangeCollaborationMode}
        onChangeSandboxMode={onChangeSandboxMode}
        onChangeModelSelection={onChangeModelSelection}
        onOpenSettings={onOpenSettings}
        onAddContextSources={onAddContextSources}
        onRemoveContextSource={onRemoveContextSource}
        onToggleSkill={onToggleSkill}
      />
    </>
  );
}

function newTaskLaunchModeLabel(mode: NewTaskLaunchMode): string {
  return mode === "new_worktree" ? "新工作树" : "在本地处理";
}

function OfflineState({
  backendUrl,
  error,
  attempt,
  isProbing,
  onRetry,
}: {
  backendUrl?: string;
  error: string | null;
  attempt: number;
  isProbing: boolean;
  onRetry: () => void;
}) {
  return (
    <div className="empty-state offline">
      <TerminalSquare size={48} />
      <h2>正在等待本地服务</h2>
      <p>
        {import.meta.env.DEV ? (
          <>
            开发模式下本地服务由 <code>cargo run</code> 启动，首次编译或改动
            Rust 代码后可能需要几分钟；编译进度会打印在运行{" "}
            <code>pnpm dev</code> 的终端里。
          </>
        ) : (
          "本地服务正在启动。"
        )}
        此页面会自动重连，无需手动刷新。
      </p>
      <small>{backendUrl ?? "http://127.0.0.1:8787"}</small>
      <div className="offline-actions">
        <button
          className="secondary-button"
          type="button"
          disabled={isProbing}
          onClick={onRetry}
        >
          <RotateCcw size={14} className={isProbing ? "spin" : undefined} />
          {isProbing ? "连接中…" : "立即重试"}
        </button>
        <small>已尝试 {attempt + 1} 次</small>
      </div>
      {/* Early failures are just the build still running, so the raw error only
          matters once retrying has clearly stopped helping. */}
      {error && attempt >= 10 && <pre>{error}</pre>}
    </div>
  );
}

type ArtifactReference = {
  id: string;
  kind?: string;
  bytes?: number;
};

type LegacyLocalProject = {
  id: string;
  name: string;
};

type RenameTarget = {
  kind: "project" | "thread";
  id: string;
  name: string;
};

type ProjectHoverState = {
  id: string;
  name: string;
  threadCount: number;
  workspaceRoot: string | null;
  pinned: boolean;
  remoteUrl: string | null;
  left: number;
  top: number;
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

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function friendlyProviderError(message: string): string {
  if (/401|auth_failed|master_key|unauthorized/i.test(message)) {
    return "认证失败：当前 Provider 的 Base URL 拒绝了 API Key。请在设置中更新该 Provider 的密钥并测试连接。";
  }
  return message;
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

function parsePathList(value: string): string[] {
  return [
    ...new Set(
      value
        .split(/\r?\n/)
        .map((path) => path.trim())
        .filter(Boolean),
    ),
  ];
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
      bytes: readNumber(metadata.artifactBytes),
    });
  }
  if (isRecord(metadata.artifact)) {
    const nestedId = readString(metadata.artifact.id);
    if (nestedId) {
      refs.push({
        id: nestedId,
        kind: readString(metadata.artifact.kind),
        bytes: readNumber(metadata.artifact.bytes),
      });
    }
  }
  return refs;
}

function artifactReferencesFromText(text: string): ArtifactReference[] {
  const refs: ArtifactReference[] = [];
  const pattern =
    /\[Artifact:\s*([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12})\]/g;
  let match: RegExpExecArray | null;
  while ((match = pattern.exec(text)) !== null) {
    refs.push({ id: match[1] });
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
        contentType: "text/plain; charset=utf-8",
        bytes: ref.bytes ?? 0,
        createdAt: event.createdAt,
      },
    ];
  }
  return next;
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

function formatBytes(value: number): string {
  if (value < 1024) return `${value} B`;
  const units = ["KB", "MB", "GB"];
  let amount = value / 1024;
  let unitIndex = 0;
  while (amount >= 1024 && unitIndex < units.length - 1) {
    amount /= 1024;
    unitIndex += 1;
  }
  return `${amount.toFixed(amount >= 10 ? 0 : 1)} ${units[unitIndex]}`;
}

const toolTabMenuItems: Array<{
  kind: "terminal" | "browser" | "files";
  shortcut: string | null;
}> = [
  { kind: "terminal", shortcut: null },
  { kind: "browser", shortcut: "Ctrl+T" },
  { kind: "files", shortcut: "Ctrl+P" },
];

const toolStageLauncherKinds: Array<{
  kind: Exclude<ToolTabKind, "preview" | "side-task">;
  label: string;
}> = [
  { kind: "diff", label: "代码审阅" },
  { kind: "terminal", label: "终端" },
  { kind: "browser", label: "浏览器" },
  { kind: "files", label: "文件" },
  { kind: "evaluations", label: "评测" },
];

function toolTabTitle(kind: ToolTabKind): string {
  switch (kind) {
    case "files":
      return "文件";
    case "terminal":
      return "终端";
    case "diff":
      return "审查";
    case "extensions":
      return "Plugins";
    case "sandbox":
      return "沙箱";
    case "evaluations":
      return "评测";
    case "browser":
      return "浏览器";
    case "computer":
      return "电脑";
    case "side-task":
      return "侧边任务";
    case "preview":
      return "预览";
  }
}

function toolTabIcon(kind: ToolTabKind): typeof Folder {
  switch (kind) {
    case "files":
      return Folder;
    case "terminal":
      return TerminalSquare;
    case "diff":
      return GitBranch;
    case "extensions":
      return Plug;
    case "sandbox":
      return Box;
    case "evaluations":
      return Workflow;
    case "browser":
      return Globe2;
    case "computer":
      return Monitor;
    case "side-task":
      return CirclePlus;
    case "preview":
      return FileCode2;
  }
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
  return /\.(?:avif|bmp|gif|ico|jpe?g|pdf|png|svg|webp|xlsm?|xlsx|xltx)$/i.test(
    path,
  );
}

const MAX_THREAD_TITLE_CHARS = 20;

function threadTitleNeedsSummary(prompt: string): boolean {
  return Array.from(prompt.trim()).length > MAX_THREAD_TITLE_CHARS;
}

function threadTitleFromPrompt(prompt: string): string {
  const title = prompt.trim();
  const chars = Array.from(title);
  if (chars.length <= MAX_THREAD_TITLE_CHARS) return title;
  const singleLineTitle = Array.from(title.replace(/\s+/g, " "));
  return `${singleLineTitle.slice(0, MAX_THREAD_TITLE_CHARS - 1).join("")}…`;
}

function workspaceName(workspaceRoot: string): string {
  const trimmed = workspaceRoot.replace(/[\\\/]+$/, "");
  const parts = trimmed.split(/[\\\/]/).filter(Boolean);
  return parts.at(-1) || workspaceRoot;
}

function formatRelativeThreadTime(value: string): string {
  const updatedAt = Date.parse(value);
  if (Number.isNaN(updatedAt)) return "";

  const elapsedMinutes = Math.max(
    0,
    Math.floor((Date.now() - updatedAt) / (60 * 1_000)),
  );
  if (elapsedMinutes < 1) return "刚刚";
  if (elapsedMinutes < 60) return `${elapsedMinutes} 分`;

  const elapsedHours = Math.floor(elapsedMinutes / 60);
  if (elapsedHours < 24) return `${elapsedHours} 小时`;

  const elapsedDays = Math.floor(elapsedHours / 24);
  return `${elapsedDays} 天`;
}

function workspaceRootKey(workspaceRoot: string): string {
  let unified = workspaceRoot.trim().replace(/\\/g, "/");
  if (/^\/\/\?\/unc\//i.test(unified)) {
    unified = `//${unified.slice(8)}`;
  } else if (/^\/\/\?\//.test(unified)) {
    unified = unified.slice(4);
  }
  const prefix = unified.startsWith("//")
    ? "//"
    : unified.startsWith("/")
      ? "/"
      : "";
  const remainder = unified.slice(prefix.length).replace(/^\/+/, "");
  const normalized = `${prefix}${remainder.replace(/\/+/g, "/")}`;
  const withoutTrailingSeparators =
    normalized.length > prefix.length
      ? normalized.replace(/\/+$/, "")
      : normalized;
  return withoutTrailingSeparators.toLowerCase();
}

function compactRemoteLabel(remoteUrl: string): string {
  const scpRemote = remoteUrl.match(/^[^@]+@([^:]+):(.+)$/);
  if (scpRemote) {
    return `${scpRemote[1]}/${scpRemote[2].replace(/\.git$/, "")}`;
  }
  try {
    const parsed = new URL(remoteUrl);
    const pathname = parsed.pathname.replace(/^\//, "").replace(/\.git$/, "");
    return pathname ? `${parsed.host}/${pathname}` : parsed.host;
  } catch {
    return remoteUrl;
  }
}
