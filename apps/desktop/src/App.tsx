import {
  Fragment,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
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
  ArrowUp,
  Bot,
  Box,
  BriefcaseBusiness,
  Check,
  ChevronDown,
  CircleHelp,
  Cloud,
  Clock3,
  Code2,
  ExternalLink,
  FileCode2,
  FileText,
  Folder,
  FolderOpen,
  GitBranch,
  GitPullRequest,
  GitFork,
  Globe2,
  Hand,
  Laptop,
  Loader2,
  Menu,
  MessageCircle,
  MoreHorizontal,
  PanelRight,
  PanelLeftClose,
  PanelLeftOpen,
  Paperclip,
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
  Square,
  SquarePen,
  Table2,
  TerminalSquare,
  Trash2,
  X,
} from "lucide-react";
import { ApiClient } from "./api/client";
import type { StreamHandle } from "./api/client";
import { LogViewer } from "./components/LogViewer";
import {
  ApprovalDialog,
  type ApprovalRequest,
} from "./components/ApprovalDialog";
import { PreviewHost } from "./components/PreviewHost";
import { RightContextRail } from "./components/RightContextRail";
import { TurnActivityTimeline } from "./components/TurnActivityTimeline";
import { WebPreviewSurface } from "./components/WebPreviewSurface";
import { WorkbenchPanel, type WorkbenchTab } from "./components/WorkbenchPanel";
import {
  deleteProviderApiKey,
  getRecentWorkspaces,
  listSecretSources,
  loadPlatformInfo,
  openPath,
  selectContextFiles,
  selectWorkspace,
  setProviderApiKey,
} from "./platform";
import type {
  AgentEvent,
  AppSettings,
  ArtifactContent,
  ArtifactDescriptor,
  ContextStatus,
  ContextSourceFile,
  ExperienceMode,
  McpServerInput,
  McpServerView,
  Message,
  MessagePart,
  KeyringMetadata,
  PlatformInfo,
  Project,
  ProviderHealth,
  ProviderHealthCheckResult,
  ProviderKind,
  ProviderSettings,
  PreviewTarget,
  RecentWorkspace,
  SandboxDescriptor,
  SecretSources,
  SkillDescriptor,
  SubagentRun,
  TerminalEvent,
  TerminalSession,
  Thread,
  ThreadMcpServerView,
  WorkspaceDiff,
  WorkspaceDiffHunk,
  WorkspaceDiffHunkAction,
  WorkspaceEntry,
  WorkspaceFilePreview,
  WorkspaceTree,
} from "./types";

type ServerStatus = "checking" | "online" | "offline";

type ToolTabKind = WorkbenchTab | "browser" | "preview";

type ToolTab = {
  id: string;
  kind: ToolTabKind;
  title: string;
  previewTarget?: PreviewTarget;
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

const workspaceLayoutStorageKey = "opentopia.workspace-layout.v1";
const experienceModeStorageKey = "opentopia.experience-mode.v1";
const workspaceThreePaneBreakpoint = 1120;
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

export function App() {
  const [platform, setPlatform] = useState<PlatformInfo | null>(null);
  const [client, setClient] = useState<ApiClient | null>(null);
  const [serverStatus, setServerStatus] = useState<ServerStatus>("checking");
  const [serverError, setServerError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [projects, setProjects] = useState<Project[]>([]);
  const [threads, setThreads] = useState<Thread[]>([]);
  const [activeThreadId, setActiveThreadId] = useState<string | null>(null);
  const [experienceMode, setExperienceMode] =
    useState<ExperienceMode>(readExperienceMode);
  const [selectedWorkspaceRoot, setSelectedWorkspaceRoot] = useState<
    string | null
  >(null);
  const [workspaceError, setWorkspaceError] = useState<string | null>(null);
  const [isPickingWorkspace, setIsPickingWorkspace] = useState(false);
  const [messages, setMessages] = useState<Message[]>([]);
  const [events, setEvents] = useState<AgentEvent[]>([]);
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
  const [isSending, setIsSending] = useState(false);
  const [activeTurnId, setActiveTurnId] = useState<string | null>(null);
  const [cancellingTurnId, setCancellingTurnId] = useState<string | null>(null);
  const [pendingApprovalIds, setPendingApprovalIds] = useState<string[]>([]);
  const [decidingApprovalId, setDecidingApprovalId] = useState<string | null>(
    null,
  );
  const [approvalDecisionError, setApprovalDecisionError] = useState<
    string | null
  >(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [providerHealth, setProviderHealth] = useState<ProviderHealth[]>([]);
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
  const [toolTabs, setToolTabs] = useState<ToolTab[]>([]);
  const [activeToolTabId, setActiveToolTabId] = useState<string | null>(null);
  const [conversationCollapsed, setConversationCollapsed] = useState(false);
  const [draftProjectId, setDraftProjectId] = useState<string | null>(null);
  const [renameTarget, setRenameTarget] = useState<RenameTarget | null>(null);
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

  const activeThread = useMemo(
    () => threads.find((thread) => thread.id === activeThreadId) ?? null,
    [threads, activeThreadId],
  );
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
  const activeToolTab = useMemo(
    () => toolTabs.find((tab) => tab.id === activeToolTabId) ?? null,
    [activeToolTabId, toolTabs],
  );
  const pendingApprovalQueue = useMemo(
    () =>
      events
        .filter(
          (event): event is AgentEvent & { payload: ApprovalRequest } =>
            event.payload.type === "approval_requested" &&
            pendingApprovalIds.includes(event.payload.approval_id),
        )
        .sort((a, b) => a.seq - b.seq),
    [events, pendingApprovalIds],
  );
  const activeApproval = pendingApprovalQueue[0]?.payload ?? null;

  useEffect(() => {
    if (!activeApproval) return;
    setConversationCollapsed(false);
    setActiveToolTabId(null);
    setActionError((current) =>
      current &&
      /resolve the pending approval before starting another turn/i.test(current)
        ? null
        : current,
    );
  }, [activeApproval?.approval_id]);

  const workspaceLayout = useMemo(
    () =>
      resolveWorkspaceLayout(
        workspaceLayoutPreferences,
        workspaceWidth,
        Boolean(activeToolTab),
        conversationCollapsed,
      ),
    [
      activeToolTab,
      conversationCollapsed,
      workspaceLayoutPreferences,
      workspaceWidth,
    ],
  );
  const workspaceStyle = {
    "--workspace-left-width": `${workspaceLayout.left}px`,
    "--workspace-right-width": `${workspaceLayout.right}px`,
  } as CSSProperties;

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

  useEffect(
    () => () => {
      if (workspaceResizeFrameRef.current !== null) {
        window.cancelAnimationFrame(workspaceResizeFrameRef.current);
      }
    },
    [],
  );

  useEffect(() => {
    if (!client) return;
    let cancelled = false;
    void client
      .listSkills(currentWorkspaceRoot)
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
  }, [client, currentWorkspaceRoot]);

  const ingestEvent = useCallback((event: AgentEvent) => {
    setEvents((current) => {
      if (current.some((item) => item.id === event.id)) return current;
      return [...current, event].sort((a, b) => a.seq - b.seq);
    });

    if (event.payload.type === "assistant_message") {
      const assistantMessage = event.payload.message;
      setMessages((current) => {
        if (current.some((message) => message.id === assistantMessage.id))
          return current;
        return [...current, assistantMessage];
      });
    }

    if (event.payload.type === "approval_requested") {
      const approvalId = event.payload.approval_id;
      setApprovalDecisionError(null);
      setConversationCollapsed(false);
      setPendingApprovalIds((current) =>
        current.includes(approvalId) ? current : [...current, approvalId],
      );
    }

    if (event.payload.type === "error") {
      setActionError(
        `Agent 请求失败：${friendlyProviderError(event.payload.message)}`,
      );
    }

    if (event.payload.type === "turn_started" && event.turnId) {
      setActiveTurnId(event.turnId);
      setCancellingTurnId(null);
    } else if (
      event.payload.type === "turn_finished" ||
      event.payload.type === "turn_suspended" ||
      event.payload.type === "turn_cancelled" ||
      event.payload.type === "error"
    ) {
      setActiveTurnId((current) =>
        !event.turnId || current === event.turnId ? null : current,
      );
      setCancellingTurnId((current) =>
        !event.turnId || current === event.turnId ? null : current,
      );
    }

    if (event.payload.type === "tool_call_finished") {
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
      }));
    }

    if (event.payload.type === "subagent_updated") {
      const run = event.payload.run;
      setSubagentRuns((current) =>
        [run, ...current.filter((item) => item.id !== run.id)].sort(
          (left, right) => right.createdAt.localeCompare(left.createdAt),
        ),
      );
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
    void loadPlatformInfo().then(async (info) => {
      if (cancelled) return;
      const nextClient = new ApiClient(info.backendUrl);
      setPlatform(info);
      setClient(nextClient);

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
      } catch (error) {
        if (cancelled) return;
        setServerStatus("offline");
        setServerError(error instanceof Error ? error.message : String(error));
      }
    });

    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    setPendingApprovalIds([]);
    setDecidingApprovalId(null);
    setApprovalDecisionError(null);
    setActiveTurnId(null);
    setCancellingTurnId(null);
    if (!client || !activeThreadId) return;
    let cancelled = false;
    let source: StreamHandle | null = null;

    void (async () => {
      const [
        loadedMessages,
        loadedEvents,
        turnStatus,
        pendingApprovals,
        loadedSubagents,
      ] = await Promise.all([
        client.listMessages(activeThreadId),
        client.listEvents(activeThreadId),
        client.getTurnStatus(activeThreadId),
        client.listPendingApprovals(activeThreadId),
        client.listSubagents(activeThreadId),
      ]);
      if (cancelled) return;
      setMessages(loadedMessages);
      setEvents(loadedEvents);
      setActiveTurnId(
        turnStatus?.status === "running" || turnStatus?.status === "cancelling"
          ? turnStatus.turnId
          : null,
      );
      setPendingApprovalIds(
        pendingApprovals.map((approval) => approval.approvalId),
      );
      setSubagentRuns(loadedSubagents);
      const since = loadedEvents.at(-1)?.seq;
      source = client.openEventStream(activeThreadId, since, ingestEvent);
    })().catch((error) => {
      if (!cancelled)
        setServerError(error instanceof Error ? error.message : String(error));
    });

    return () => {
      cancelled = true;
      source?.close();
    };
  }, [activeThreadId, client, ingestEvent]);

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
      `移除项目“${project.name}”？所属任务会归档，可在“已归档”中恢复。`,
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
      setActionError(`移除项目失败：${errorMessage(error)}`);
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
    if (kind === "preview") return;
    const id = `tool-${kind}`;
    setToolTabs((current) =>
      current.some((tab) => tab.id === id)
        ? current
        : [...current, { id, kind, title: toolTabTitle(kind) }],
    );
    setActiveToolTabId(id);
    setConversationCollapsed(false);
  }

  function openPreviewTab(
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
    setConversationCollapsed(false);
  }

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
        openPreviewTab(
          activeThread.id,
          { type: "workspace", path: entry.path },
          entry.name,
        );
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

  async function saveSettings(input: {
    providers?: ProviderSettings[];
    activeProviderId?: string;
    providerKind?: ProviderKind;
    baseUrl?: string;
    model?: string;
    apiKeySource?: string;
    permissionMode?: "chat" | "read_only" | "auto" | "approve" | "full_access";
    sandbox?: AppSettings["sandbox"];
  }) {
    if (!client) return;
    setIsSavingSettings(true);
    try {
      const updated = await client.updateSettings(input);
      setSettings(updated);
      setProviderHealth(await client.getProviderHealth());
      if (activeThread) setSandbox(await client.getSandbox(activeThread.id));
    } catch (error) {
      setActionError(`保存设置失败：${errorMessage(error)}`);
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

  async function addContextSources() {
    setActionError(null);
    try {
      const result = await selectContextFiles({
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

  async function spawnSubagent(name: string, input: string) {
    if (!client || !activeThread) return;
    setActionError(null);
    try {
      await client.spawnSubagent(activeThread.id, {
        name,
        input,
        agentType: "default",
        forkTurns: "all",
        parentTurnId: activeTurnId ?? undefined,
        depth: 1,
      });
    } catch (error) {
      setActionError(`启动子智能体失败：${errorMessage(error)}`);
      throw error;
    }
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

  async function createThread(initialPrompt?: string): Promise<Thread | null> {
    if (!client) return null;
    if (newTaskLaunchMode === "new_worktree") {
      setActionError(
        "“新工作树”启动模式尚未接入线程创建；请选择“在本地处理”后继续。",
      );
      return null;
    }
    const directCommand = parseDirectToolCommand(initialPrompt ?? "");
    if (!directCommand && isLegacyDirectToolCommand(initialPrompt ?? "")) {
      setActionError("/run and /read require an argument.");
      return null;
    }
    if (
      directCommand &&
      (contextSources.length > 0 || selectedSkillIds.length > 0)
    ) {
      setActionError("Direct tool commands cannot include agent context.");
      return null;
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
    if (!project?.workspaceRoot) return null;

    setIsSending(
      Boolean(initialPrompt?.trim()) ||
        contextSources.length > 0 ||
        selectedSkillIds.length > 0,
    );
    setActionError(null);
    try {
      const thread = await client.createThread({
        title: initialPrompt?.trim()
          ? threadTitleFromPrompt(initialPrompt)
          : project.name,
        workspaceRoot: project.workspaceRoot,
        projectId: project.id,
        experienceMode,
      });
      setThreads((current) => [thread, ...current]);
      setActiveThreadId(thread.id);
      setSelectedWorkspaceRoot(thread.workspaceRoot);
      setDraftProjectId(null);
      setToolTabs([]);
      setActiveToolTabId(null);
      if (directCommand) {
        await runDirectToolCommand(thread.id, directCommand);
        setComposer("");
      } else if (
        initialPrompt?.trim() ||
        contextSources.length > 0 ||
        selectedSkillIds.length > 0
      ) {
        const { message, turnId } = await client.sendMessage(
          thread.id,
          initialPrompt?.trim() ?? "",
          contextSources.map((source) => source.path),
          selectedSkillIds,
        );
        setMessages([message]);
        if (turnId) setActiveTurnId(turnId);
        setComposer("");
        setContextSources([]);
        setSelectedSkillIds([]);
      }
      return thread;
    } catch (error) {
      setActionError(`创建任务失败：${errorMessage(error)}`);
      return null;
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

  async function submitMessage() {
    if (
      !client ||
      !activeThread ||
      (!composer.trim() &&
        contextSources.length === 0 &&
        selectedSkillIds.length === 0) ||
      isSending ||
      activeTurnId ||
      activeApproval
    )
      return;
    const directCommand = parseDirectToolCommand(composer);
    if (!directCommand && isLegacyDirectToolCommand(composer)) {
      setActionError("/run and /read require an argument.");
      return;
    }
    if (
      directCommand &&
      (contextSources.length > 0 || selectedSkillIds.length > 0)
    ) {
      setActionError("Direct tool commands cannot include agent context.");
      return;
    }
    setIsSending(true);
    try {
      if (directCommand) {
        await runDirectToolCommand(activeThread.id, directCommand);
        setComposer("");
        return;
      }
      const { message, turnId } = await client.sendMessage(
        activeThread.id,
        composer.trim(),
        contextSources.map((source) => source.path),
        selectedSkillIds,
      );
      setMessages((current) => [...current, message]);
      if (turnId) setActiveTurnId(turnId);
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
    } catch (error) {
      setActionError(`Could not send message: ${errorMessage(error)}`);
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
      const summary = await client.compactContext(activeThread.id);
      setContextStatus((current) => ({
        budget: current?.budget ?? {
          totalTokens: 128000,
          usedTokens: 0,
          messageCount: messages.length,
          estimatedUsage: 0,
        },
        latestSummary: summary,
      }));
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

  async function storeProviderApiKey(
    providerId: string,
    value: string,
  ): Promise<KeyringMetadata | null> {
    if (isSavingSecret) return null;
    setIsSavingSecret(true);
    setServerError(null);
    try {
      const metadata = await setProviderApiKey(providerId, value);
      setSecretSources(await listSecretSources());
      return metadata;
    } catch (error) {
      setServerError(error instanceof Error ? error.message : String(error));
      return null;
    } finally {
      setIsSavingSecret(false);
    }
  }

  async function removeProviderApiKey(
    providerId: string,
  ): Promise<KeyringMetadata | null> {
    if (isSavingSecret) return null;
    setIsSavingSecret(true);
    setServerError(null);
    try {
      const metadata = await deleteProviderApiKey(providerId);
      setSecretSources(await listSecretSources());
      return metadata;
    } catch (error) {
      setServerError(error instanceof Error ? error.message : String(error));
      return null;
    } finally {
      setIsSavingSecret(false);
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
        setProviderHealth(await client.getProviderHealth());
      }
      const result = await client.testProviderConnection(providerId);
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
      preferenceKey: isLeft
        ? "left"
        : activeToolTab
          ? "toolRight"
          : "contextRight",
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
      isLeft ? "left" : activeToolTab ? "toolRight" : "contextRight",
      clampPanelSize(next, min, max),
    );
  }

  function resetWorkspacePanelSize(side: WorkspaceResizeSide) {
    const key: keyof WorkspaceLayoutPreferences =
      side === "left" ? "left" : activeToolTab ? "toolRight" : "contextRight";
    setWorkspaceLayoutPreferences((current) => {
      if (current[key] === undefined) return current;
      const next = { ...current };
      delete next[key];
      return next;
    });
  }

  return (
    <div className="app-shell">
      <TopBar />
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
        className={`workspace ${activeToolTab ? "with-tool-stage" : ""} ${conversationCollapsed ? "tool-only" : ""} ${workspaceResizeSide ? "is-resizing" : ""}`}
        style={workspaceStyle}
      >
        <Sidebar
          projects={projects}
          threads={threads}
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
          onToggleProjectPinned={(project) => void toggleProjectPinned(project)}
          onSelectProject={beginProjectDraft}
          onNewThreadForProject={handleNewThreadForProject}
          onRenameThread={(thread) =>
            setRenameTarget({
              kind: "thread",
              id: thread.id,
              name: thread.title,
            })
          }
          onArchiveThread={(thread) => void archiveThread(thread)}
          onRestoreThread={(thread) => void restoreThread(thread)}
          onOpenThreadWorkspace={(workspaceRoot) =>
            void openWorkspaceRoot(workspaceRoot)
          }
          onOpenExtensions={() => openToolTab("extensions")}
          onSettings={() => setSettingsOpen(true)}
        />
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
          onPointerMove={(event) => continueWorkspaceResize("left", event)}
          onPointerUp={(event) => finishWorkspaceResize("left", event)}
          onPointerCancel={(event) => finishWorkspaceResize("left", event)}
          onLostPointerCapture={(event) => finishWorkspaceResize("left", event)}
          onDoubleClick={() => resetWorkspacePanelSize("left")}
          onKeyDown={(event) => resizeWorkspaceWithKeyboard("left", event)}
        />
        <section
          className={`center-pane ${activeApproval ? "has-approval" : ""}`}
          id="workspace-center-pane"
        >
          <ThreadHeader
            thread={activeThread}
            onOpenLocation={() =>
              activeThread && void openWorkspaceRoot(activeThread.workspaceRoot)
            }
            onOpenTool={openToolTab}
            onRename={() =>
              activeThread &&
              setRenameTarget({
                kind: "thread",
                id: activeThread.id,
                name: activeThread.title,
              })
            }
            onArchive={() => activeThread && void archiveThread(activeThread)}
          />
          {serverStatus === "offline" ? (
            <OfflineState
              backendUrl={platform?.backendUrl}
              error={serverError}
            />
          ) : activeThread ? (
            <>
              <MessageList
                messages={messages}
                events={events}
                activeTurnId={activeTurnId}
                threadId={activeThread.id}
                artifacts={artifacts}
                onOpenArtifact={(artifactId) =>
                  void openArtifact(activeThread.id, artifactId)
                }
              />
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
                    void decideApproval(activeApproval.approval_id, approved)
                  }
                />
              ) : (
                <Composer
                  value={composer}
                  isSending={isSending}
                  isRunning={Boolean(activeTurnId)}
                  isCancelling={
                    Boolean(activeTurnId) && cancellingTurnId === activeTurnId
                  }
                  model={
                    settings?.providers.find(
                      (provider) => provider.id === settings.activeProviderId,
                    )?.model ?? "Model"
                  }
                  permissionMode={settings?.permissionMode ?? "auto"}
                  sandboxMode={
                    settings?.sandbox.sandboxMode ?? "workspace-write"
                  }
                  contextSources={contextSources}
                  skills={skills}
                  selectedSkillIds={selectedSkillIds}
                  workspaceRoot={null}
                  projectName={null}
                  projects={projects}
                  canOpenThreadTools
                  onChange={setComposer}
                  onSubmit={submitMessage}
                  onCancel={() => void cancelTurn()}
                  onOpenTool={openToolTab}
                  onPickWorkspace={() => void chooseWorkspace()}
                  onSelectProject={selectProject}
                  onChangePermissionMode={changeExecutionPreset}
                  onChangeSandboxMode={changeSandboxMode}
                  onAddContextSources={() => void addContextSources()}
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
              model={
                settings?.providers.find(
                  (provider) => provider.id === settings.activeProviderId,
                )?.model ?? "Model"
              }
              permissionMode={settings?.permissionMode ?? "auto"}
              sandboxMode={settings?.sandbox.sandboxMode ?? "workspace-write"}
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
              onOpenTool={openToolTab}
              onChangePermissionMode={changeExecutionPreset}
              onChangeSandboxMode={changeSandboxMode}
              onAddContextSources={() => void addContextSources()}
              onRemoveContextSource={removeContextSource}
              onToggleSkill={toggleSkill}
              onSubmit={() => void createThread(composer)}
            />
          )}
        </section>
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
          onPointerMove={(event) => continueWorkspaceResize("right", event)}
          onPointerUp={(event) => finishWorkspaceResize("right", event)}
          onPointerCancel={(event) => finishWorkspaceResize("right", event)}
          onLostPointerCapture={(event) =>
            finishWorkspaceResize("right", event)
          }
          onDoubleClick={() => resetWorkspacePanelSize("right")}
          onKeyDown={(event) => resizeWorkspaceWithKeyboard("right", event)}
        />
        <RightPanel
          client={client}
          toolTabs={toolTabs}
          activeToolTab={activeToolTab}
          conversationCollapsed={conversationCollapsed}
          thread={activeThread}
          workspaceRoot={currentWorkspaceRoot}
          messages={messages}
          events={events.filter(
            (event) =>
              event.payload.type !== "approval_requested" ||
              pendingApprovalIds.includes(event.payload.approval_id),
          )}
          subagentRuns={subagentRuns}
          terminalEvents={terminalEvents}
          terminalSession={terminalSession}
          workspaceTree={workspaceTree}
          filePreview={filePreview}
          workspaceDiff={workspaceDiff}
          sandbox={sandbox}
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
          onRevertDiffFile={(path) => void revertDiffFile(path)}
          onApplyDiffHunk={(hunk, action) => void applyDiffHunk(hunk, action)}
          onGetArtifact={(threadId, artifactId) =>
            getArtifact(threadId, artifactId)
          }
          onOpenToolTab={openToolTab}
          onActivateToolTab={setActiveToolTabId}
          onCloseToolTab={closeToolTab}
          onToggleConversation={() =>
            setConversationCollapsed((current) => !current)
          }
          onAddContextSources={() => void addContextSources()}
          onSpawnSubagent={spawnSubagent}
          onCancelSubagent={(runId) => void cancelSubagent(runId)}
        />
      </main>
      {settingsOpen && (
        <SettingsPanel
          platform={platform}
          settings={settings}
          providerHealth={providerHealth}
          providerTest={providerTest}
          secretSources={secretSources}
          isSaving={isSavingSettings}
          isSavingSecret={isSavingSecret}
          onSave={(input) => void saveSettings(input)}
          onTestProvider={(providerId, providers) =>
            void testProviderConnection(providerId, providers)
          }
          onStoreProviderApiKey={storeProviderApiKey}
          onDeleteProviderApiKey={removeProviderApiKey}
          onOpenLogs={() => {
            setSettingsOpen(false);
            setLogViewerOpen(true);
          }}
          onClose={() => setSettingsOpen(false)}
        />
      )}
      {logViewerOpen && <LogViewer onClose={() => setLogViewerOpen(false)} />}
      {renameTarget && (
        <RenameDialog
          target={renameTarget}
          onSubmit={submitRename}
          onClose={() => setRenameTarget(null)}
        />
      )}
    </div>
  );
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

function TopBar() {
  return (
    <header className="topbar">
      <div className="window-menu">
        <button
          className="window-app-button"
          disabled
          title="应用菜单 · 未实现"
        >
          <Menu size={14} />
        </button>
        <button className="window-nav-button" disabled title="后退 · 未实现">
          <ArrowLeft size={14} />
        </button>
        <button className="window-nav-button" disabled title="前进 · 未实现">
          <ArrowRight size={14} />
        </button>
        {[
          ["文件", "文件菜单 · 未实现"],
          ["编辑", "编辑菜单 · 未实现"],
          ["视图", "视图菜单 · 未实现"],
          ["帮助", "帮助菜单 · 未实现"],
        ].map(([label, title]) => (
          <button
            className="window-menu-item"
            disabled
            key={label}
            title={title}
          >
            {label}
          </button>
        ))}
      </div>
    </header>
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
  ): Promise<KeyringMetadata | null>;
  onDeleteProviderApiKey(providerId: string): Promise<KeyringMetadata | null>;
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
      network: "deny",
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
        kind: "openai_compatible",
        baseUrl: "https://api.openai.com/v1",
        model: "gpt-4.1-mini",
        temperature: 0.2,
        maxOutputTokens: null,
        contextWindowTokens: 128000,
        reasoningEffort: null,
        storeResponses: false,
        parallelToolCalls: false,
        promptCacheKey: null,
        rolloutBudget: null,
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
                      <option value="openai_compatible">
                        OpenAI Compatible
                      </option>
                      <option value="openai_responses">OpenAI Responses</option>
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
                        value={editingProvider.temperature}
                        onChange={(event) =>
                          updateProvider(
                            editingProvider.id,
                            "temperature",
                            Number(event.target.value),
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
                        value={editingProvider.contextWindowTokens}
                        onChange={(event) =>
                          updateProvider(
                            editingProvider.id,
                            "contextWindowTokens",
                            Number(event.target.value),
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
                        <span>Store Responses state</span>
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
                              ).then((metadata) => {
                                if (!metadata) return;
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
                                (metadata) => {
                                  if (!metadata) return;
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
  projects,
  threads,
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
  onArchiveThread,
  onRestoreThread,
  onOpenExtensions,
  onSettings,
}: {
  projects: Project[];
  threads: Thread[];
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
  onArchiveThread(thread: Thread): void;
  onRestoreThread(thread: Thread): void;
  onOpenExtensions(): void;
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
          <button
            className="sidebar-icon-button"
            disabled
            title="搜索 · 未实现"
            aria-label="搜索"
          >
            <Search size={15} />
          </button>
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
          <button onClick={onOpenExtensions} title="管理 MCP 扩展">
            <Plug size={15} />
            <span>插件</span>
            <small>MCP</small>
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
                            onClick={() => {
                              onRenameProject(project);
                              setMoreMenuProjectId(null);
                            }}
                          >
                            <Pencil size={14} />
                            <span>重命名项目</span>
                          </button>
                          <button
                            role="menuitem"
                            onClick={() => {
                              onToggleProjectPinned(project);
                              setMoreMenuProjectId(null);
                            }}
                          >
                            <Pin size={14} />
                            <span>
                              {project.pinned ? "取消固定" : "固定项目"}
                            </span>
                          </button>
                          <div className="tool-popover-separator" />
                          <button
                            role="menuitem"
                            onClick={() => {
                              onRemoveProject(project);
                              setMoreMenuProjectId(null);
                            }}
                          >
                            <X size={14} />
                            <span>从最近项目移除</span>
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
                        key={thread.id}
                        thread={thread}
                        onSelect={() => onSelect(thread.id)}
                        onOpenWorkspace={() =>
                          onOpenThreadWorkspace(thread.workspaceRoot)
                        }
                        onRename={() => onRenameThread(thread)}
                        onArchive={() => onArchiveThread(thread)}
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
                      key={thread.id}
                      thread={thread}
                      onSelect={() => onSelect(thread.id)}
                      onOpenWorkspace={() =>
                        onOpenThreadWorkspace(thread.workspaceRoot)
                      }
                      onRename={() => onRenameThread(thread)}
                      onArchive={() => onArchiveThread(thread)}
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
                      key={thread.id}
                      thread={thread}
                      onSelect={() => onRestoreThread(thread)}
                      onOpenWorkspace={() =>
                        onOpenThreadWorkspace(thread.workspaceRoot)
                      }
                      onRename={() => onRenameThread(thread)}
                      onArchive={() => undefined}
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

function SidebarThreadRow({
  thread,
  active,
  archived = false,
  onSelect,
  onOpenWorkspace,
  onRename,
  onArchive,
  onRestore,
}: {
  thread: Thread;
  active: boolean;
  archived?: boolean;
  onSelect(): void;
  onOpenWorkspace(): void;
  onRename(): void;
  onArchive(): void;
  onRestore?(): void;
}) {
  const [menuOpen, setMenuOpen] = useState(false);
  const menuRef = useDismissiblePopover(menuOpen, () => setMenuOpen(false));

  return (
    <div className={`thread-row-wrap ${menuOpen ? "menu-open" : ""}`}>
      <button
        className={`thread-row ${active ? "active" : ""}`}
        onClick={onSelect}
        title={thread.title}
      >
        <span>{thread.title}</span>
      </button>
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
                onOpenWorkspace();
                setMenuOpen(false);
              }}
            >
              <FolderOpen size={14} />
              <span>在文件管理器中打开</span>
            </button>
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
            <button disabled title="Git 工作树管理尚未实现">
              <GitFork size={14} />
              <span>创建工作树</span>
              <small>未实现</small>
            </button>
            {archived ? (
              <button
                role="menuitem"
                onClick={() => {
                  onRestore?.();
                  setMenuOpen(false);
                }}
              >
                <RotateCcw size={14} />
                <span>恢复到项目</span>
              </button>
            ) : (
              <button
                role="menuitem"
                onClick={() => {
                  onArchive();
                  setMenuOpen(false);
                }}
              >
                <Archive size={14} />
                <span>归档</span>
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
  onOpenLocation,
  onOpenTool,
  onRename,
  onArchive,
}: {
  thread: Thread | null;
  onOpenLocation(): void;
  onOpenTool(kind: ToolTabKind): void;
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
      </div>
    </div>
  );
}

function MessageList({
  messages,
  events,
  activeTurnId,
  threadId,
  artifacts,
  onOpenArtifact,
}: {
  messages: Message[];
  events: AgentEvent[];
  activeTurnId: string | null;
  threadId: string;
  artifacts: ArtifactDescriptor[];
  onOpenArtifact(artifactId: string): void;
}) {
  const streamingText = activeTurnId
    ? events
        .filter(
          (event) =>
            event.turnId === activeTurnId &&
            event.payload.type === "model_delta",
        )
        .map((event) =>
          event.payload.type === "model_delta" ? event.payload.text : "",
        )
        .join("")
    : "";
  const eventsByTurn = new Map<string, AgentEvent[]>();
  const turnIdByUserMessage = new Map<string, string>();
  for (const event of events) {
    if (event.turnId) {
      const current = eventsByTurn.get(event.turnId) ?? [];
      current.push(event);
      eventsByTurn.set(event.turnId, current);
    }
    if (event.turnId && event.payload.type === "turn_started") {
      turnIdByUserMessage.set(event.payload.user_message_id, event.turnId);
    }
  }
  const anchoredTurnIds = new Set(turnIdByUserMessage.values());
  const orphanTurnErrors = events.filter(
    (event) =>
      event.payload.type === "error" &&
      (!event.turnId || !anchoredTurnIds.has(event.turnId)),
  );
  return (
    <div className="message-list">
      {messages.length === 0 ? (
        <div className="empty-thread">
          <Bot size={42} />
          <h2>等待第一个任务指令</h2>
          <p>当前任务尚未产生消息。</p>
        </div>
      ) : (
        messages.map((message) => {
          const turnId =
            message.role === "user"
              ? turnIdByUserMessage.get(message.id)
              : undefined;
          const turnEvents = turnId ? (eventsByTurn.get(turnId) ?? []) : [];
          return (
            <Fragment key={message.id}>
              <MessageBubble
                message={message}
                threadId={threadId}
                artifacts={artifacts}
                onOpenArtifact={onOpenArtifact}
              />
              {turnId && (
                <TurnActivityTimeline
                  events={turnEvents}
                  isActive={activeTurnId === turnId}
                  formatError={friendlyProviderError}
                />
              )}
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
      {streamingText && (
        <article className="message assistant streaming-message">
          <div className="message-body">
            <p className="message-text">{streamingText}</p>
          </div>
        </article>
      )}
    </div>
  );
}

function MessageBubble({
  message,
  threadId,
  artifacts,
  onOpenArtifact,
}: {
  message: Message;
  threadId: string;
  artifacts: ArtifactDescriptor[];
  onOpenArtifact(artifactId: string): void;
}) {
  return (
    <article className={`message ${message.role}`}>
      <div className="message-body">
        {message.parts.map((part, index) => (
          <MessagePartView
            key={index}
            part={part}
            threadId={threadId}
            artifacts={artifacts}
            onOpenArtifact={onOpenArtifact}
          />
        ))}
      </div>
    </article>
  );
}

function MessagePartView({
  part,
  threadId,
  artifacts,
  onOpenArtifact,
}: {
  part: MessagePart;
  threadId: string;
  artifacts: ArtifactDescriptor[];
  onOpenArtifact(artifactId: string): void;
}) {
  if (part.type === "text") {
    const refs = artifactReferencesFromText(part.text);
    return (
      <>
        <p className="message-text">{part.text}</p>
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
        <Paperclip size={12} />
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
  if (part.type === "tool_call")
    return <pre>{JSON.stringify(part.call, null, 2)}</pre>;
  return (
    <>
      <pre>{part.result.output}</pre>
      <MessageArtifactLinks
        refs={collectArtifactReferences(
          part.result.metadata,
          part.result.output,
        )}
        threadId={threadId}
        artifacts={artifacts}
        onOpenArtifact={onOpenArtifact}
      />
    </>
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

function Composer({
  value,
  isSending,
  isRunning,
  isCancelling,
  model,
  permissionMode,
  sandboxMode,
  contextSources,
  skills,
  selectedSkillIds,
  workspaceRoot,
  projectName,
  projects,
  launchMode,
  canOpenThreadTools = false,
  onChange,
  onSubmit,
  onCancel,
  onOpenTool,
  onPickWorkspace,
  onSelectProject,
  onChangeLaunchMode,
  onChangePermissionMode,
  onChangeSandboxMode,
  onAddContextSources,
  onRemoveContextSource,
  onToggleSkill,
}: {
  value: string;
  isSending: boolean;
  isRunning: boolean;
  isCancelling: boolean;
  model: string;
  permissionMode: AppSettings["permissionMode"];
  sandboxMode: AppSettings["sandbox"]["sandboxMode"];
  contextSources: ContextSourceFile[];
  skills: SkillDescriptor[];
  selectedSkillIds: string[];
  workspaceRoot: string | null;
  projectName: string | null;
  projects: Project[];
  launchMode?: NewTaskLaunchMode;
  canOpenThreadTools?: boolean;
  onChange(value: string): void;
  onSubmit(): void;
  onCancel(): void;
  onOpenTool(kind: ToolTabKind): void;
  onPickWorkspace(): void;
  onSelectProject(projectId: string): void;
  onChangeLaunchMode?(mode: NewTaskLaunchMode): void;
  onChangePermissionMode(mode: ExecutionPermissionMode): void;
  onChangeSandboxMode(mode: AppSettings["sandbox"]["sandboxMode"]): void;
  onAddContextSources(): void;
  onRemoveContextSource(path: string): void;
  onToggleSkill(skillId: string): void;
}) {
  const [openMenu, setOpenMenu] = useState<
    | "actions"
    | "skills"
    | "permission"
    | "model"
    | "workspace"
    | "environment"
    | null
  >(null);
  const popoverRef = useDismissiblePopover(Boolean(openMenu), () =>
    setOpenMenu(null),
  );

  return (
    <div
      className={`composer ${workspaceRoot || projectName ? "has-context" : ""} ${contextSources.length || selectedSkillIds.length ? "has-sources" : ""}`}
      ref={popoverRef}
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
                  <div className="tool-popover launch-mode-popover" role="menu">
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
                      className={launchMode === "new_worktree" ? "active" : ""}
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
                  <div className="tool-popover environment-popover" role="menu">
                    {sandboxModeOptions.map((option) => (
                      <button
                        className={sandboxMode === option.value ? "active" : ""}
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
      {(contextSources.length > 0 || selectedSkillIds.length > 0) && (
        <div className="composer-sources" aria-label="已添加来源">
          {contextSources.map((source) => (
            <span
              className="composer-source"
              key={source.path}
              title={source.path}
            >
              <Paperclip size={12} />
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
      <textarea
        value={value}
        aria-label="消息"
        placeholder="请求后续更改"
        onChange={(event) => onChange(event.target.value)}
        onKeyDown={(event) => {
          if (
            event.key === "Enter" &&
            !event.altKey &&
            !event.nativeEvent.isComposing &&
            !event.repeat
          ) {
            event.preventDefault();
            onSubmit();
          }
        }}
      />
      <div className="composer-toolbar">
        <div className="composer-menu-wrap">
          <button
            className="composer-icon-button"
            type="button"
            title="添加上下文或打开工具"
            aria-label="添加上下文或打开工具"
            aria-expanded={openMenu === "actions"}
            onClick={() =>
              setOpenMenu((current) =>
                current === "actions" ? null : "actions",
              )
            }
          >
            <Plus size={16} />
          </button>
          {openMenu === "actions" && (
            <div className="tool-popover composer-actions-popover" role="menu">
              <button
                role="menuitem"
                disabled={!canOpenThreadTools}
                title={canOpenThreadTools ? undefined : "创建任务后可浏览文件"}
                onClick={() => {
                  onPickWorkspace();
                  setOpenMenu(null);
                }}
              >
                <Folder size={14} />
                <span>选择工作区</span>
              </button>
              <button
                role="menuitem"
                onClick={() => {
                  onAddContextSources();
                  setOpenMenu(null);
                }}
              >
                <Paperclip size={14} />
                <span>添加文件或图片</span>
              </button>
              <button
                role="menuitem"
                disabled={!canOpenThreadTools}
                title={canOpenThreadTools ? undefined : "创建任务后可打开终端"}
                onClick={() => {
                  onOpenTool("terminal");
                  setOpenMenu(null);
                }}
              >
                <TerminalSquare size={14} />
                <span>终端</span>
                {!canOpenThreadTools && <small>创建任务后</small>}
              </button>
              <button
                role="menuitem"
                disabled={!canOpenThreadTools}
                title={canOpenThreadTools ? undefined : "创建任务后可审查变更"}
                onClick={() => {
                  onOpenTool("diff");
                  setOpenMenu(null);
                }}
              >
                <GitBranch size={14} />
                <span>审查变更</span>
                {!canOpenThreadTools && <small>创建任务后</small>}
              </button>
              <button
                role="menuitem"
                disabled={!canOpenThreadTools}
                title={
                  canOpenThreadTools ? undefined : "创建任务后可打开浏览器"
                }
                onClick={() => {
                  onOpenTool("browser");
                  setOpenMenu(null);
                }}
              >
                <Globe2 size={14} />
                <span>Browser</span>
                {!canOpenThreadTools && <small>创建任务后</small>}
              </button>
              <button role="menuitem" onClick={() => setOpenMenu("skills")}>
                <Plug size={14} />
                <span>Skills</span>
                <small>{selectedSkillIds.length || skills.length}</small>
              </button>
            </div>
          )}
          {openMenu === "skills" && (
            <div
              className="tool-popover composer-actions-popover skills-popover"
              role="menu"
            >
              <div className="tool-popover-note">
                <strong>Skills</strong>
                <span>最多为当前 Turn 选择 5 个</span>
              </div>
              {skills.length ? (
                skills.map((skill) => {
                  const selected = selectedSkillIds.includes(skill.id);
                  return (
                    <button
                      className={selected ? "active" : ""}
                      key={skill.id}
                      role="menuitemcheckbox"
                      aria-checked={selected}
                      disabled={!selected && selectedSkillIds.length >= 5}
                      title={skill.description || skill.path}
                      onClick={() => onToggleSkill(skill.id)}
                    >
                      {selected ? <Check size={13} /> : <Plug size={13} />}
                      <span>{skill.name}</span>
                      <small>
                        {skill.scope === "workspace" ? "项目" : "用户"}
                      </small>
                    </button>
                  );
                })
              ) : (
                <button disabled>
                  <Plug size={13} />
                  <span>未发现 Skills</span>
                </button>
              )}
              <div className="tool-popover-separator" />
              <button onClick={() => setOpenMenu("actions")}>
                <ArrowLeft size={13} />
                <span>返回</span>
              </button>
            </div>
          )}
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
        <div className="composer-menu-wrap composer-meta-wrap">
          <button
            className="composer-meta"
            type="button"
            aria-expanded={openMenu === "model"}
            onClick={() =>
              setOpenMenu((current) => (current === "model" ? null : "model"))
            }
          >
            <span title={model}>{model}</span>
            <span>默认推理</span>
            <ChevronDown size={12} />
          </button>
          {openMenu === "model" && (
            <div className="tool-popover model-popover" role="menu">
              <div className="tool-popover-note">
                <strong>{model}</strong>
                <span>当前 Provider 模型</span>
              </div>
              <button disabled title="单任务模型与推理强度尚未实现">
                <Activity size={14} />
                <span>模型与推理强度</span>
                <small>使用全局配置</small>
              </button>
            </div>
          )}
        </div>
      </div>
      <button
        className={`send-button${isRunning ? " is-running" : ""}`}
        type="button"
        disabled={
          isRunning
            ? isCancelling
            : isSending ||
              (!value.trim() &&
                contextSources.length === 0 &&
                selectedSkillIds.length === 0)
        }
        onClick={isRunning ? onCancel : onSubmit}
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
    </div>
  );
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

function RightPanel({
  client,
  toolTabs,
  activeToolTab,
  conversationCollapsed,
  thread,
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
  onDecideApproval,
  onRefreshWorkbench,
  onOpenWorkspacePath,
  onOpenWorkspaceEntry,
  onToggleThreadMcp,
  onCreateMcpServer,
  onUpdateMcpServer,
  onRestartMcpServer,
  onDeleteMcpServer,
  onOpenWorkspace,
  onEnsureTerminalSession,
  onWriteTerminalSession,
  onResizeTerminalSession,
  onCloseTerminalSession,
  onCompactContext,
  onOpenArtifact,
  onRevertDiffFile,
  onApplyDiffHunk,
  onGetArtifact,
  onOpenToolTab,
  onActivateToolTab,
  onCloseToolTab,
  onToggleConversation,
  onAddContextSources,
  onSpawnSubagent,
  onCancelSubagent,
}: {
  client: ApiClient | null;
  toolTabs: ToolTab[];
  activeToolTab: ToolTab | null;
  conversationCollapsed: boolean;
  thread: Thread | null;
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
  onDecideApproval(approvalId: string, approved: boolean): void;
  onRefreshWorkbench(): void;
  onOpenWorkspacePath(path?: string): void;
  onOpenWorkspaceEntry(entry: WorkspaceEntry): void;
  onToggleThreadMcp(serverId: string, enabled: boolean): void;
  onCreateMcpServer(input: McpServerInput): Promise<void>;
  onUpdateMcpServer(serverId: string, input: McpServerInput): Promise<void>;
  onRestartMcpServer(serverId: string): Promise<void>;
  onDeleteMcpServer(serverId: string): Promise<void>;
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
  onRevertDiffFile(path: string): void;
  onApplyDiffHunk(
    hunk: WorkspaceDiffHunk,
    action: WorkspaceDiffHunkAction,
  ): void;
  onGetArtifact(threadId: string, artifactId: string): Promise<ArtifactContent>;
  onOpenToolTab(kind: ToolTabKind): void;
  onActivateToolTab(tabId: string): void;
  onCloseToolTab(tabId: string): void;
  onToggleConversation(): void;
  onAddContextSources(): void;
  onSpawnSubagent(name: string, input: string): Promise<void>;
  onCancelSubagent(runId: string): void;
}) {
  const renderWorkbench = (
    mode: "panel" | "stage",
    activeTab?: WorkbenchTab,
  ) => (
    <WorkbenchPanel
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
      onDecideApproval={onDecideApproval}
      onRefreshWorkbench={onRefreshWorkbench}
      onOpenWorkspacePath={onOpenWorkspacePath}
      onOpenWorkspaceEntry={onOpenWorkspaceEntry}
      onToggleThreadMcp={onToggleThreadMcp}
      onCreateMcpServer={onCreateMcpServer}
      onUpdateMcpServer={onUpdateMcpServer}
      onRestartMcpServer={onRestartMcpServer}
      onDeleteMcpServer={onDeleteMcpServer}
      onOpenPath={onOpenWorkspace}
      onEnsureTerminalSession={onEnsureTerminalSession}
      onWriteTerminalSession={onWriteTerminalSession}
      onResizeTerminalSession={onResizeTerminalSession}
      onCloseTerminalSession={onCloseTerminalSession}
      onCompactContext={onCompactContext}
      onOpenArtifact={onOpenArtifact}
      onRevertDiffFile={onRevertDiffFile}
      onApplyDiffHunk={onApplyDiffHunk}
      onGetArtifact={onGetArtifact}
    />
  );

  if (activeToolTab) {
    return (
      <aside className="right-panel tool-stage" id="workspace-right-panel">
        <ToolTabStrip
          tabs={toolTabs}
          activeTabId={activeToolTab.id}
          onActivate={onActivateToolTab}
          onClose={onCloseToolTab}
          onOpen={onOpenToolTab}
          conversationCollapsed={conversationCollapsed}
          onToggleConversation={onToggleConversation}
        />
        <div className="tool-stage-body">
          {activeToolTab.kind === "browser" ? (
            <WebPreviewSurface
              client={client}
              threadId={thread?.id ?? null}
              events={events}
              pendingApprovalIds={pendingApprovalIds}
              decidingApprovalId={decidingApprovalId}
              onDecideApproval={onDecideApproval}
            />
          ) : activeToolTab.kind === "preview" &&
            activeToolTab.previewTarget ? (
            <PreviewHost
              client={client}
              threadId={thread?.id ?? null}
              workspaceRoot={workspaceRoot}
              target={activeToolTab.previewTarget}
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
      className="right-panel context-rail-shell"
      id="workspace-right-panel"
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
        onAddSource={onAddContextSources}
        onSpawnSubagent={onSpawnSubagent}
        onCancelSubagent={onCancelSubagent}
        onGitChanged={onRefreshWorkbench}
      />
    </aside>
  );
}

function ToolTabStrip({
  tabs,
  activeTabId,
  onActivate,
  onClose,
  onOpen,
  conversationCollapsed,
  onToggleConversation,
}: {
  tabs: ToolTab[];
  activeTabId: string;
  onActivate(tabId: string): void;
  onClose(tabId: string): void;
  onOpen(kind: ToolTabKind): void;
  conversationCollapsed: boolean;
  onToggleConversation(): void;
}) {
  const [menuOpen, setMenuOpen] = useState(false);
  const menuRef = useDismissiblePopover(menuOpen, () => setMenuOpen(false));

  function open(kind: ToolTabKind) {
    onOpen(kind);
    setMenuOpen(false);
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
              role="tab"
              aria-selected={tab.id === activeTabId}
            >
              <button
                className="tool-tab-main"
                type="button"
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
      <button
        className="tool-tab-layout-toggle"
        type="button"
        title={conversationCollapsed ? "显示对话" : "隐藏对话"}
        aria-label={conversationCollapsed ? "显示对话" : "隐藏对话"}
        onClick={onToggleConversation}
      >
        {conversationCollapsed ? (
          <PanelLeftOpen size={14} />
        ) : (
          <PanelLeftClose size={14} />
        )}
      </button>
      <div className="tool-tab-add-wrap" ref={menuRef}>
        <button
          className="tool-tab-add"
          type="button"
          title="打开工具"
          aria-label="打开工具"
          aria-expanded={menuOpen}
          onClick={() => setMenuOpen((current) => !current)}
        >
          <Plus size={14} />
        </button>
        {menuOpen && (
          <div className="tool-popover tool-tab-add-popover" role="menu">
            {toolTabKinds.map((kind) => {
              const Icon = toolTabIcon(kind);
              return (
                <button key={kind} role="menuitem" onClick={() => open(kind)}>
                  <Icon size={14} />
                  <span>{toolTabTitle(kind)}</span>
                </button>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}

function NewTaskState({
  value,
  workspaceRoot,
  projectName,
  projects,
  model,
  permissionMode,
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
  onOpenTool,
  onChangePermissionMode,
  onChangeSandboxMode,
  onAddContextSources,
  onRemoveContextSource,
  onToggleSkill,
  onSubmit,
}: {
  value: string;
  workspaceRoot: string | null;
  projectName: string | null;
  projects: Project[];
  model: string;
  permissionMode: AppSettings["permissionMode"];
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
  onOpenTool(kind: ToolTabKind): void;
  onChangePermissionMode(mode: ExecutionPermissionMode): void;
  onChangeSandboxMode(mode: AppSettings["sandbox"]["sandboxMode"]): void;
  onAddContextSources(): void;
  onRemoveContextSource(path: string): void;
  onToggleSkill(skillId: string): void;
  onSubmit(): void;
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
        model={model}
        permissionMode={permissionMode}
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
        onOpenTool={onOpenTool}
        onPickWorkspace={onPickWorkspace}
        onSelectProject={onSelectProject}
        onChangeLaunchMode={onChangeLaunchMode}
        onChangePermissionMode={onChangePermissionMode}
        onChangeSandboxMode={onChangeSandboxMode}
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
}: {
  backendUrl?: string;
  error: string | null;
}) {
  return (
    <div className="empty-state offline">
      <TerminalSquare size={48} />
      <h2>Local server is offline</h2>
      <p>Start the Rust server, then reload the desktop app.</p>
      <code>cargo run -p opentopia-server</code>
      <small>{backendUrl ?? "http://127.0.0.1:8787"}</small>
      {error && <pre>{error}</pre>}
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
    network: sandbox.network === "allow" ? "inherit" : sandbox.network,
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

const toolTabKinds: ToolTabKind[] = [
  "files",
  "terminal",
  "diff",
  "extensions",
  "sandbox",
  "browser",
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
      return "MCP";
    case "sandbox":
      return "沙箱";
    case "browser":
      return "浏览器";
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
    case "browser":
      return Globe2;
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

function threadTitleFromPrompt(prompt: string): string {
  const title = prompt.replace(/\s+/g, " ").trim();
  return title.length > 32 ? `${title.slice(0, 31)}…` : title;
}

function workspaceName(workspaceRoot: string): string {
  const trimmed = workspaceRoot.replace(/[\\\/]+$/, "");
  const parts = trimmed.split(/[\\\/]/).filter(Boolean);
  return parts.at(-1) || workspaceRoot;
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
