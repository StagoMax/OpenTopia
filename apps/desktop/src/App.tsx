import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Activity,
  Archive,
  ArrowLeft,
  ArrowRight,
  Bot,
  Box,
  Check,
  ChevronDown,
  CircleHelp,
  Cloud,
  Clock3,
  ExternalLink,
  FileCode2,
  FileText,
  Folder,
  FolderOpen,
  GitBranch,
  Github,
  GitPullRequest,
  GitFork,
  Globe2,
  Loader2,
  Menu,
  MoreHorizontal,
  PanelRight,
  PanelLeftClose,
  PanelLeftOpen,
  Pencil,
  Plug,
  Plus,
  Search,
  Send,
  Settings,
  Square,
  TerminalSquare,
  Trash2,
  Users,
  X,
} from "lucide-react";
import { ApiClient } from "./api/client";
import type { StreamHandle } from "./api/client";
import { LogViewer } from "./components/LogViewer";
import { detectLanguage, MonacoEditor } from "./components/MonacoEditor";
import { WorkbenchPanel, type WorkbenchTab } from "./components/WorkbenchPanel";
import {
  deleteSecret,
  getRecentWorkspaces,
  listSecretSources,
  loadPlatformInfo,
  openPath,
  removeRecentWorkspace,
  saveRecentWorkspace,
  selectWorkspace,
  setSecret,
} from "./platform";
import type {
  AgentEvent,
  AppSettings,
  ArtifactContent,
  ArtifactDescriptor,
  ContextStatus,
  McpServerView,
  Message,
  MessagePart,
  PlatformInfo,
  ProviderHealth,
  ProviderHealthCheckResult,
  ProviderKind,
  RecentWorkspace,
  SandboxDescriptor,
  SecretSources,
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

type ToolTabKind = WorkbenchTab | "browser";

type ToolTab = {
  id: string;
  kind: ToolTabKind;
  title: string;
};

type ArtifactPreviewState =
  | { status: "loading"; artifactId: string }
  | { status: "ready"; artifactId: string; content: ArtifactContent }
  | { status: "error"; artifactId: string; message: string };

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
  const [threads, setThreads] = useState<Thread[]>([]);
  const [activeThreadId, setActiveThreadId] = useState<string | null>(null);
  const [selectedWorkspaceRoot, setSelectedWorkspaceRoot] = useState<
    string | null
  >(null);
  const [recentWorkspaces, setRecentWorkspaces] = useState<RecentWorkspace[]>(
    [],
  );
  const [workspaceError, setWorkspaceError] = useState<string | null>(null);
  const [isPickingWorkspace, setIsPickingWorkspace] = useState(false);
  const [messages, setMessages] = useState<Message[]>([]);
  const [events, setEvents] = useState<AgentEvent[]>([]);
  const [terminalEvents, setTerminalEvents] = useState<TerminalEvent[]>([]);
  const [terminalSession, setTerminalSession] =
    useState<TerminalSession | null>(null);
  const [composer, setComposer] = useState("");
  const [isSending, setIsSending] = useState(false);
  const [activeTurnId, setActiveTurnId] = useState<string | null>(null);
  const [pendingApprovalIds, setPendingApprovalIds] = useState<string[]>([]);
  const [decidingApprovalId, setDecidingApprovalId] = useState<string | null>(
    null,
  );
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
  const [artifactPreview, setArtifactPreview] =
    useState<ArtifactPreviewState | null>(null);
  const [toolTabs, setToolTabs] = useState<ToolTab[]>([]);
  const [activeToolTabId, setActiveToolTabId] = useState<string | null>(null);
  const [conversationCollapsed, setConversationCollapsed] = useState(false);
  const [draftProjectName, setDraftProjectName] = useState<string | null>(null);

  const activeThread = useMemo(
    () => threads.find((thread) => thread.id === activeThreadId) ?? null,
    [threads, activeThreadId],
  );
  const currentWorkspaceRoot =
    selectedWorkspaceRoot ?? activeThread?.workspaceRoot ?? null;
  const activeToolTab = useMemo(
    () => toolTabs.find((tab) => tab.id === activeToolTabId) ?? null,
    [activeToolTabId, toolTabs],
  );

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
      setPendingApprovalIds((current) =>
        current.includes(approvalId) ? current : [...current, approvalId],
      );
    }

    if (event.payload.type === "turn_started" && event.turnId) {
      setActiveTurnId(event.turnId);
    } else if (
      event.payload.type === "turn_finished" ||
      event.payload.type === "turn_suspended" ||
      event.payload.type === "turn_cancelled" ||
      event.payload.type === "error"
    ) {
      setActiveTurnId((current) =>
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

      let loadedRecent: RecentWorkspace[] = [];
      try {
        loadedRecent = await getRecentWorkspaces();
        if (cancelled) return;
        setRecentWorkspaces(loadedRecent);
        setSelectedWorkspaceRoot(
          (current) => current ?? loadedRecent[0]?.workspaceRoot ?? null,
        );
      } catch (error) {
        if (cancelled) return;
        setWorkspaceError(
          error instanceof Error ? error.message : String(error),
        );
      }

      try {
        await nextClient.health();
        const [loadedThreads, loadedSettings, loadedHealth, loadedMcp] =
          await Promise.all([
            nextClient.listThreads(),
            nextClient.getSettings(),
            nextClient.getProviderHealth(),
            nextClient.listMcpServers(),
          ]);
        if (cancelled) return;
        setThreads(loadedThreads);
        setSettings(loadedSettings);
        setProviderHealth(loadedHealth);
        setMcpServers(loadedMcp);
        setActiveThreadId((current) => current ?? loadedThreads[0]?.id ?? null);
        setSelectedWorkspaceRoot(
          (current) =>
            current ??
            loadedThreads[0]?.workspaceRoot ??
            loadedRecent[0]?.workspaceRoot ??
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
    if (!client || !activeThreadId) return;
    let cancelled = false;
    let source: StreamHandle | null = null;

    void (async () => {
      const [loadedMessages, loadedEvents, turnStatus, pendingApprovals] =
        await Promise.all([
          client.listMessages(activeThreadId),
          client.listEvents(activeThreadId),
          client.getTurnStatus(activeThreadId),
          client.listPendingApprovals(activeThreadId),
        ]);
      if (cancelled) return;
      setMessages(loadedMessages);
      setEvents(loadedEvents);
      setActiveTurnId(turnStatus?.turnId ?? null);
      setPendingApprovalIds(
        pendingApprovals.map((approval) => approval.approvalId),
      );
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
    setDraftProjectName(null);
    if (thread?.workspaceRoot) setSelectedWorkspaceRoot(thread.workspaceRoot);
  }

  function beginNewThread() {
    setActiveThreadId(null);
    setMessages([]);
    setEvents([]);
    setComposer("");
    setActiveTurnId(null);
    setPendingApprovalIds([]);
    setToolTabs([]);
    setActiveToolTabId(null);
    setConversationCollapsed(false);
    setDraftProjectName(
      currentWorkspaceRoot ? workspaceName(currentWorkspaceRoot) : null,
    );
  }

  function beginProjectDraft(projectName: string) {
    beginNewThread();
    setSelectedWorkspaceRoot(null);
    setDraftProjectName(projectName);
  }

  function handleNewThreadForProject(
    workspaceRoot: string | null,
    projectName: string,
  ) {
    if (workspaceRoot) {
      void selectRecentWorkspace(workspaceRoot);
      beginNewThread();
    } else {
      beginProjectDraft(projectName);
    }
  }

  function openToolTab(kind: ToolTabKind) {
    const id = `tool-${kind}`;
    setToolTabs((current) =>
      current.some((tab) => tab.id === id)
        ? current
        : [...current, { id, kind, title: toolTabTitle(kind) }],
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

  async function chooseWorkspace(): Promise<string | null> {
    setIsPickingWorkspace(true);
    setWorkspaceError(null);
    try {
      const result = await selectWorkspace({
        defaultPath: currentWorkspaceRoot ?? undefined,
      });
      if (result.canceled) return null;

      setSelectedWorkspaceRoot(result.workspaceRoot);
      if (!activeThread) {
        setDraftProjectName((current) => current ?? result.workspace.name);
      }
      setRecentWorkspaces(result.recentWorkspaces);
      return result.workspaceRoot;
    } catch (error) {
      setWorkspaceError(error instanceof Error ? error.message : String(error));
      return null;
    } finally {
      setIsPickingWorkspace(false);
    }
  }

  async function selectRecentWorkspace(workspaceRoot: string) {
    setWorkspaceError(null);
    setSelectedWorkspaceRoot(workspaceRoot);
    setDraftProjectName(workspaceName(workspaceRoot));
    try {
      setRecentWorkspaces(await saveRecentWorkspace(workspaceRoot));
    } catch (error) {
      setWorkspaceError(error instanceof Error ? error.message : String(error));
    }
  }

  async function forgetRecentWorkspace(workspaceRoot: string) {
    setWorkspaceError(null);
    try {
      const nextRecentWorkspaces = await removeRecentWorkspace(workspaceRoot);
      setRecentWorkspaces(nextRecentWorkspaces);
      if (selectedWorkspaceRoot === workspaceRoot) {
        setSelectedWorkspaceRoot(
          nextRecentWorkspaces[0]?.workspaceRoot ??
            activeThread?.workspaceRoot ??
            null,
        );
      }
    } catch (error) {
      setWorkspaceError(error instanceof Error ? error.message : String(error));
    }
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
        setFilePreview(
          await client.readWorkspaceFile(activeThread.id, entry.path),
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

  async function saveSettings(input: {
    providers?: {
      id: string;
      kind: ProviderKind;
      baseUrl: string;
      model: string;
      apiKeySource: string;
      apiKeyConfigured: boolean;
      healthStatus?: string | null;
    }[];
    activeProviderId?: string;
    providerKind?: ProviderKind;
    baseUrl?: string;
    model?: string;
    apiKeySource?: string;
    permissionMode: "chat" | "read_only" | "auto" | "approve" | "full_access";
  }) {
    if (!client) return;
    setIsSavingSettings(true);
    try {
      const updated = await client.updateSettings(input);
      setSettings(updated);
      setProviderHealth(await client.getProviderHealth());
    } finally {
      setIsSavingSettings(false);
    }
  }

  async function createThread(initialPrompt?: string): Promise<Thread | null> {
    if (!client) return null;
    const workspaceRoot = currentWorkspaceRoot ?? (await chooseWorkspace());
    if (!workspaceRoot) return null;

    setIsSending(Boolean(initialPrompt?.trim()));
    try {
      const thread = await client.createThread({
        title: initialPrompt?.trim()
          ? threadTitleFromPrompt(initialPrompt)
          : workspaceName(workspaceRoot),
        workspaceRoot,
      });
      setThreads((current) => [thread, ...current]);
      setActiveThreadId(thread.id);
      setSelectedWorkspaceRoot(thread.workspaceRoot);
      setToolTabs([]);
      setActiveToolTabId(null);
      if (initialPrompt?.trim()) {
        const message = await client.sendMessage(
          thread.id,
          initialPrompt.trim(),
        );
        setMessages([message]);
        setComposer("");
      }
      try {
        setRecentWorkspaces(await saveRecentWorkspace(thread.workspaceRoot));
      } catch (error) {
        setWorkspaceError(
          error instanceof Error ? error.message : String(error),
        );
      }
      return thread;
    } catch (error) {
      setServerError(error instanceof Error ? error.message : String(error));
      return null;
    } finally {
      setIsSending(false);
    }
  }

  async function submitMessage() {
    if (
      !client ||
      !activeThread ||
      !composer.trim() ||
      isSending ||
      activeTurnId
    )
      return;
    setIsSending(true);
    try {
      const message = await client.sendMessage(
        activeThread.id,
        composer.trim(),
      );
      setMessages((current) => [...current, message]);
      setComposer("");
    } finally {
      setIsSending(false);
    }
  }

  async function cancelTurn() {
    if (!client || !activeThread || !activeTurnId) return;
    try {
      await client.cancelTurn(activeThread.id, activeTurnId);
    } catch (error) {
      setServerError(error instanceof Error ? error.message : String(error));
    }
  }

  async function decideApproval(approvalId: string, approved: boolean) {
    if (!client || !activeThread || decidingApprovalId) return;
    setDecidingApprovalId(approvalId);
    try {
      await client.decideApproval(activeThread.id, approvalId, approved);
      setPendingApprovalIds((current) =>
        current.filter((id) => id !== approvalId),
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

  async function openArtifact(threadId: string, artifactId: string) {
    if (!client) return;
    setArtifactPreview({ status: "loading", artifactId });
    try {
      const content = await client.getArtifact(threadId, artifactId);
      setArtifactPreview({ status: "ready", artifactId, content });
    } catch (error) {
      setArtifactPreview({
        status: "error",
        artifactId,
        message: error instanceof Error ? error.message : String(error),
      });
    }
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

  async function storeProviderApiKey(value: string) {
    if (!secretSources?.keyring || isSavingSecret) return;
    setIsSavingSecret(true);
    setServerError(null);
    try {
      await setSecret(secretSources.keyring.providerApiKeySourceId, value);
      setSecretSources(await listSecretSources());
    } catch (error) {
      setServerError(error instanceof Error ? error.message : String(error));
    } finally {
      setIsSavingSecret(false);
    }
  }

  async function removeProviderApiKey() {
    if (!secretSources?.keyring || isSavingSecret) return;
    setIsSavingSecret(true);
    setServerError(null);
    try {
      await deleteSecret(secretSources.keyring.providerApiKeySourceId);
      setSecretSources(await listSecretSources());
    } catch (error) {
      setServerError(error instanceof Error ? error.message : String(error));
    } finally {
      setIsSavingSecret(false);
    }
  }

  async function testProviderConnection(providerId: string) {
    if (!client || providerTest?.status === "testing") return;
    setProviderTest({ providerId, status: "testing" });
    try {
      const result = await client.testProviderConnection(providerId);
      setProviderTest({ providerId, status: "complete", result });
    } catch (error) {
      setProviderTest({
        providerId,
        status: "complete",
        result: {
          reachable: false,
          modelAvailable: false,
          error: error instanceof Error ? error.message : String(error),
        },
      });
    }
  }

  return (
    <div className="app-shell">
      <TopBar />
      <main
        className={`workspace ${activeToolTab ? "with-tool-stage" : ""} ${conversationCollapsed ? "tool-only" : ""}`}
      >
        <Sidebar
          threads={threads}
          activeThreadId={activeThreadId}
          selectedWorkspaceRoot={currentWorkspaceRoot}
          selectedDraftProjectName={draftProjectName}
          recentWorkspaces={recentWorkspaces}
          workspaceError={workspaceError}
          isPickingWorkspace={isPickingWorkspace}
          onSelect={selectThread}
          onNew={beginNewThread}
          onPickWorkspace={() => void chooseWorkspace()}
          onSelectWorkspace={(workspaceRoot) =>
            void selectRecentWorkspace(workspaceRoot)
          }
          onForgetWorkspace={(workspaceRoot) =>
            void forgetRecentWorkspace(workspaceRoot)
          }
          onSelectDraftProject={beginProjectDraft}
          onNewThreadForProject={handleNewThreadForProject}
          onOpenThreadWorkspace={(workspaceRoot) =>
            void openWorkspaceRoot(workspaceRoot)
          }
          onSettings={() => setSettingsOpen(true)}
        />
        <section className="center-pane">
          <ThreadHeader
            thread={activeThread}
            onOpenLocation={() =>
              activeThread && void openWorkspaceRoot(activeThread.workspaceRoot)
            }
            onOpenTool={openToolTab}
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
              <Composer
                value={composer}
                isSending={isSending}
                isRunning={Boolean(activeTurnId)}
                model={
                  settings?.providers.find(
                    (provider) => provider.id === settings.activeProviderId,
                  )?.model ?? "Model"
                }
                permissionMode={settings?.permissionMode ?? "auto"}
                workspaceRoot={null}
                projectName={null}
                recentWorkspaces={recentWorkspaces}
                canOpenThreadTools
                onChange={setComposer}
                onSubmit={submitMessage}
                onCancel={() => void cancelTurn()}
                onOpenTool={openToolTab}
                onPickWorkspace={() => void chooseWorkspace()}
                onSelectWorkspace={(workspaceRoot) =>
                  void selectRecentWorkspace(workspaceRoot)
                }
                onChangePermissionMode={(permissionMode) =>
                  void saveSettings({ permissionMode })
                }
              />
            </>
          ) : (
            <NewTaskState
              value={composer}
              workspaceRoot={currentWorkspaceRoot}
              projectName={draftProjectName}
              recentWorkspaces={recentWorkspaces}
              model={
                settings?.providers.find(
                  (provider) => provider.id === settings.activeProviderId,
                )?.model ?? "Model"
              }
              permissionMode={settings?.permissionMode ?? "auto"}
              isSending={isSending}
              onChange={setComposer}
              onPickWorkspace={() => void chooseWorkspace()}
              onSelectWorkspace={(workspaceRoot) =>
                void selectRecentWorkspace(workspaceRoot)
              }
              onOpenTool={openToolTab}
              onChangePermissionMode={(permissionMode) =>
                void saveSettings({ permissionMode })
              }
              onSubmit={() => void createThread(composer)}
            />
          )}
        </section>
        <RightPanel
          toolTabs={toolTabs}
          activeToolTab={activeToolTab}
          conversationCollapsed={conversationCollapsed}
          thread={activeThread}
          workspaceRoot={currentWorkspaceRoot}
          events={events.filter(
            (event) =>
              event.payload.type !== "approval_requested" ||
              pendingApprovalIds.includes(event.payload.approval_id),
          )}
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
          onDecideApproval={decideApproval}
          onRefreshWorkbench={() => void refreshWorkbench()}
          onOpenWorkspacePath={(path) => void openWorkspacePath(path)}
          onOpenWorkspaceEntry={(entry) => void openWorkspaceEntry(entry)}
          onToggleThreadMcp={(serverId, enabled) =>
            void toggleThreadMcp(serverId, enabled)
          }
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
          onTestProvider={(providerId) =>
            void testProviderConnection(providerId)
          }
          onStoreProviderApiKey={(value) => void storeProviderApiKey(value)}
          onDeleteProviderApiKey={() => void removeProviderApiKey()}
          onOpenLogs={() => {
            setSettingsOpen(false);
            setLogViewerOpen(true);
          }}
          onClose={() => setSettingsOpen(false)}
        />
      )}
      {logViewerOpen && <LogViewer onClose={() => setLogViewerOpen(false)} />}
      {artifactPreview && (
        <ArtifactPreviewModal
          preview={artifactPreview}
          onOpenPath={(targetPath) => void openWorkspaceRoot(targetPath)}
          onClose={() => setArtifactPreview(null)}
        />
      )}
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
    providers?: {
      id: string;
      kind: ProviderKind;
      baseUrl: string;
      model: string;
      apiKeySource: string;
      apiKeyConfigured: boolean;
      healthStatus?: string | null;
    }[];
    activeProviderId?: string;
    providerKind?: ProviderKind;
    baseUrl?: string;
    model?: string;
    apiKeySource?: string;
    permissionMode: "chat" | "read_only" | "auto" | "approve" | "full_access";
  }): void;
  onTestProvider(providerId: string): void;
  onStoreProviderApiKey(value: string): void;
  onDeleteProviderApiKey(): void;
  onOpenLogs(): void;
  onClose(): void;
}) {
  const [providers, setProviders] = useState<
    {
      id: string;
      kind: ProviderKind;
      baseUrl: string;
      model: string;
      apiKeySource: string;
      apiKeyConfigured: boolean;
      healthStatus?: string | null;
    }[]
  >(settings?.providers ?? []);
  const [activeProviderId, setActiveProviderId] = useState(
    settings?.activeProviderId ?? providers[0]?.id ?? "default",
  );
  const [editingProviderId, setEditingProviderId] = useState<string | null>(
    null,
  );
  const [permissionMode, setPermissionMode] = useState<
    "chat" | "read_only" | "auto" | "approve" | "full_access"
  >(settings?.permissionMode ?? "auto");
  const [providerApiKey, setProviderApiKey] = useState("");

  const editingProvider =
    providers.find((p) => p.id === editingProviderId) ?? providers[0] ?? null;

  useEffect(() => {
    if (settings) {
      setProviders(settings.providers);
      setActiveProviderId(settings.activeProviderId);
      setPermissionMode(settings.permissionMode);
    }
  }, [settings]);

  function updateProvider(id: string, field: string, value: string) {
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
              onChange={(event) =>
                setPermissionMode(
                  event.target.value as
                    "chat" | "read_only" | "auto" | "approve" | "full_access",
                )
              }
            >
              <option value="chat">Chat</option>
              <option value="read_only">Read Only</option>
              <option value="auto">Auto</option>
              <option value="approve">Approve</option>
              <option value="full_access">Full Access</option>
            </select>
          </label>

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
                      onChange={(e) =>
                        updateProvider(editingProvider.id, "id", e.target.value)
                      }
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
                          e.target.value,
                        )
                      }
                    >
                      <option value="openai_compatible">
                        OpenAI Compatible
                      </option>
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
                  <label>
                    API Key Env
                    <input
                      value={editingProvider.apiKeySource}
                      onChange={(e) =>
                        updateProvider(
                          editingProvider.id,
                          "apiKeySource",
                          e.target.value,
                        )
                      }
                    />
                  </label>
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
                      onClick={() => onTestProvider(editingProvider.id)}
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
                          Desktop API key
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
                              onStoreProviderApiKey(providerApiKey);
                              setProviderApiKey("");
                            }}
                          >
                            Store key
                          </button>
                          <button
                            type="button"
                            className="secondary-button"
                            disabled={
                              isSavingSecret ||
                              !secretSources.keyring.providerApiKeyConfigured
                            }
                            onClick={onDeleteProviderApiKey}
                          >
                            Remove key
                          </button>
                          <span className="settings-provider-test-result">
                            {secretSources.keyring.providerApiKeyConfigured
                              ? "Stored for the next backend start"
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
  threads,
  activeThreadId,
  selectedWorkspaceRoot,
  selectedDraftProjectName,
  recentWorkspaces,
  workspaceError,
  isPickingWorkspace,
  onSelect,
  onNew,
  onPickWorkspace,
  onSelectWorkspace,
  onForgetWorkspace,
  onSelectDraftProject,
  onOpenThreadWorkspace,
  onNewThreadForProject,
  onSettings,
}: {
  threads: Thread[];
  activeThreadId: string | null;
  selectedWorkspaceRoot: string | null;
  selectedDraftProjectName: string | null;
  recentWorkspaces: RecentWorkspace[];
  workspaceError: string | null;
  isPickingWorkspace: boolean;
  onSelect(id: string): void;
  onNew(): void;
  onPickWorkspace(): void;
  onSelectWorkspace(workspaceRoot: string): void;
  onForgetWorkspace(workspaceRoot: string): void;
  onSelectDraftProject(projectName: string): void;
  onOpenThreadWorkspace(workspaceRoot: string): void;
  onNewThreadForProject?(workspaceRoot: string | null, projectName: string): void;
  onSettings(): void;
}) {
  const [projectMenuOpen, setProjectMenuOpen] = useState(false);
  const [newProjectOpen, setNewProjectOpen] = useState(false);
  const [newProjectName, setNewProjectName] = useState("New project");
  const [localProjects, setLocalProjects] = useState<LocalProject[]>(() =>
    readLocalProjects(),
  );
  const [expandedProjects, setExpandedProjects] = useState<Set<string>>(
    () => new Set(),
  );
  const [moreMenuProjectId, setMoreMenuProjectId] = useState<string | null>(
    null,
  );
  const moreMenuRef = useDismissiblePopover(moreMenuProjectId !== null, () =>
    setMoreMenuProjectId(null),
  );
  const projectMenuRef = useDismissiblePopover(projectMenuOpen, () =>
    setProjectMenuOpen(false),
  );

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

  const projects = useMemo(() => {
    const roots = new Map<string, { name: string; workspaceRoot: string }>();
    for (const workspace of recentWorkspaces) {
      roots.set(workspace.workspaceRoot, workspace);
    }
    for (const thread of threads) {
      if (!roots.has(thread.workspaceRoot)) {
        roots.set(thread.workspaceRoot, {
          name: workspaceName(thread.workspaceRoot),
          workspaceRoot: thread.workspaceRoot,
        });
      }
    }
    return [
      ...localProjects.map((project) => ({
        id: project.id,
        name: project.name,
        workspaceRoot: null,
      })),
      ...[...roots.values()].map((project) => ({
        id: project.workspaceRoot,
        ...project,
      })),
    ];
  }, [localProjects, recentWorkspaces, threads]);

  function createLocalProject() {
    const name = newProjectName.trim();
    if (!name) return;
    const project = { id: crypto.randomUUID(), name };
    setLocalProjects((current) => {
      const next = [project, ...current];
      writeLocalProjects(next);
      return next;
    });
    setNewProjectOpen(false);
    setProjectMenuOpen(false);
    setNewProjectName("New project");
    onSelectDraftProject(name);
  }

  function removeLocalProject(projectId: string) {
    setLocalProjects((current) => {
      const next = current.filter((project) => project.id !== projectId);
      writeLocalProjects(next);
      return next;
    });
  }

  return (
    <>
      <aside className="sidebar">
        <div className="sidebar-brand-row">
          <strong>
            <span className="brand-open">Open</span><span>Topia</span>
          </strong>
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
          <button disabled title="MCP 插件已实现，请在右侧扩展标签管理">
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
                <Plus size={14} />
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
          {projects.map((project) => {
            const projectThreads = project.workspaceRoot
              ? threads.filter(
                  (thread) => thread.workspaceRoot === project.workspaceRoot,
                )
              : [];
            const isActive = project.workspaceRoot
              ? project.workspaceRoot === selectedWorkspaceRoot
              : !activeThreadId && project.name === selectedDraftProjectName;
            const isExpanded = expandedProjects.has(project.id);
            const isMoreMenuOpen = moreMenuProjectId === project.id;
            return (
              <section
                className={`project-node ${isActive ? "active" : ""}`}
                key={project.id}
              >
                <div className="project-row">
                  <button
                    className="project-select"
                    title={project.workspaceRoot ?? project.name}
                    onClick={() => {
                      toggleExpandedProject(project.id);
                      project.workspaceRoot
                        ? onSelectWorkspace(project.workspaceRoot)
                        : onSelectDraftProject(project.name);
                    }}
                  >
                    {isExpanded ? <FolderOpen size={14} /> : <Folder size={14} />}
                    <span>{project.name}</span>
                  </button>
                  <div className="project-row-actions">
                    <button
                      className="project-new-thread"
                      title="新建对话"
                      onClick={() => {
                        onNewThreadForProject?.(
                          project.workspaceRoot,
                          project.name,
                        );
                      }}
                    >
                      <Plus size={13} />
                    </button>
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
                        <div className="tool-popover project-row-popover" role="menu">
                          <button
                            role="menuitem"
                            onClick={() => {
                              project.workspaceRoot
                                ? onForgetWorkspace(project.workspaceRoot)
                                : removeLocalProject(project.id);
                              setMoreMenuProjectId(null);
                            }}
                          >
                            <X size={14} />
                            <span>从最近项目移除</span>
                          </button>
                        </div>
                      )}
                    </div>
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
          {projects.length === 0 && (
            <p className="workspace-empty">尚未打开项目</p>
          )}
          {workspaceError && (
            <p className="workspace-error">{workspaceError}</p>
          )}
        </div>

        <div className="sidebar-footer">
          <button onClick={onSettings}>
            <Settings size={15} />
            <span>设置</span>
          </button>
          <button disabled title="帮助 · 未实现" aria-label="帮助">
            <CircleHelp size={15} />
          </button>
        </div>
      </aside>
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
              createLocalProject();
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
                disabled={!newProjectName.trim()}
              >
                保存
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
  onSelect,
  onOpenWorkspace,
}: {
  thread: Thread;
  active: boolean;
  onSelect(): void;
  onOpenWorkspace(): void;
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
            <button disabled title="线程重命名 API 尚未实现">
              <Pencil size={14} />
              <span>重命名</span>
              <small>未实现</small>
            </button>
            <button disabled title="Git 工作树管理尚未实现">
              <GitFork size={14} />
              <span>创建工作树</span>
              <small>未实现</small>
            </button>
            <button disabled title="任务归档 API 尚未实现">
              <Archive size={14} />
              <span>归档</span>
              <small>未实现</small>
            </button>
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
}: {
  thread: Thread | null;
  onOpenLocation(): void;
  onOpenTool(kind: ToolTabKind): void;
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
              <button disabled title="线程重命名 API 尚未实现">
                <Pencil size={14} />
                <span>重命名任务</span>
                <small>未实现</small>
              </button>
              <button disabled title="Git 工作树管理尚未实现">
                <GitFork size={14} />
                <span>创建工作树</span>
                <small>未实现</small>
              </button>
              <button disabled title="任务归档 API 尚未实现">
                <Archive size={14} />
                <span>归档任务</span>
                <small>未实现</small>
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
  const completedToolSteps = activeTurnId
    ? events.filter(
        (event) =>
          event.turnId === activeTurnId &&
          event.payload.type === "tool_call_finished",
      ).length
    : 0;
  return (
    <div className="message-list">
      {messages.length === 0 ? (
        <div className="empty-thread">
          <Bot size={42} />
          <h2>等待第一个任务指令</h2>
          <p>当前任务尚未产生消息。</p>
        </div>
      ) : (
        messages.map((message) => (
          <MessageBubble
            key={message.id}
            message={message}
            threadId={threadId}
            artifacts={artifacts}
            onOpenArtifact={onOpenArtifact}
          />
        ))
      )}
      {streamingText && (
        <article className="message assistant streaming-message">
          <div className="message-body">
            <p className="message-text">{streamingText}</p>
          </div>
        </article>
      )}
      {activeTurnId && (
        <div className="agent-progress" role="status">
          <Loader2 size={14} className="spin" />
          <span>{streamingText ? "正在生成回复" : "正在思考"}</span>
          {completedToolSteps > 0 && (
            <small>已完成 {completedToolSteps} 个工具步骤</small>
          )}
        </div>
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
  model,
  permissionMode,
  workspaceRoot,
  projectName,
  recentWorkspaces,
  canOpenThreadTools = false,
  onChange,
  onSubmit,
  onCancel,
  onOpenTool,
  onPickWorkspace,
  onSelectWorkspace,
  onChangePermissionMode,
}: {
  value: string;
  isSending: boolean;
  isRunning: boolean;
  model: string;
  permissionMode: AppSettings["permissionMode"];
  workspaceRoot: string | null;
  projectName: string | null;
  recentWorkspaces: RecentWorkspace[];
  canOpenThreadTools?: boolean;
  onChange(value: string): void;
  onSubmit(): void;
  onCancel(): void;
  onOpenTool(kind: ToolTabKind): void;
  onPickWorkspace(): void;
  onSelectWorkspace(workspaceRoot: string): void;
  onChangePermissionMode(mode: AppSettings["permissionMode"]): void;
}) {
  const [openMenu, setOpenMenu] = useState<
    "actions" | "permission" | "model" | "workspace" | "environment" | null
  >(null);
  const popoverRef = useDismissiblePopover(Boolean(openMenu), () =>
    setOpenMenu(null),
  );

  return (
    <div
      className={`composer ${workspaceRoot || projectName ? "has-context" : ""}`}
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
                {recentWorkspaces.map((workspace) => (
                  <button
                    key={workspace.workspaceRoot}
                    role="menuitemradio"
                    aria-checked={workspace.workspaceRoot === workspaceRoot}
                    onClick={() => {
                      onSelectWorkspace(workspace.workspaceRoot);
                      setOpenMenu(null);
                    }}
                  >
                    {workspace.workspaceRoot === workspaceRoot ? (
                      <Check size={13} />
                    ) : (
                      <Folder size={13} />
                    )}
                    <span>{workspace.name}</span>
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
              <span>本地</span>
              <ChevronDown size={11} />
            </button>
            {openMenu === "environment" && (
              <div className="tool-popover environment-popover" role="menu">
                <button
                  className="active"
                  role="menuitemradio"
                  aria-checked
                  onClick={() => setOpenMenu(null)}
                >
                  <Check size={13} />
                  <span>在本地处理</span>
                </button>
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
      <textarea
        value={value}
        aria-label="消息"
        placeholder="请求后续更改"
        onChange={(event) => onChange(event.target.value)}
        onKeyDown={(event) => {
          if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
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
                  onOpenTool("files");
                  setOpenMenu(null);
                }}
              >
                <FileText size={14} />
                <span>文件和文件夹</span>
                {!canOpenThreadTools && <small>创建任务后</small>}
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
              <button disabled title="图片附件协议尚未实现">
                <Plus size={14} />
                <span>添加图片</span>
                <small>未实现</small>
              </button>
              <button disabled title="Skill 选择协议尚未实现">
                <Plug size={14} />
                <span>Skills</span>
                <small>未实现</small>
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
              {permissionModeOptions.map((option) => (
                <button
                  className={permissionMode === option.value ? "active" : ""}
                  key={option.value}
                  role="menuitemradio"
                  aria-checked={permissionMode === option.value}
                  onClick={() => {
                    onChangePermissionMode(option.value);
                    setOpenMenu(null);
                  }}
                >
                  {permissionMode === option.value ? (
                    <Check size={13} />
                  ) : (
                    <span className="menu-icon-spacer" />
                  )}
                  <span>{option.label}</span>
                </button>
              ))}
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
        className="send-button"
        disabled={!isRunning && (isSending || !value.trim())}
        onClick={isRunning ? onCancel : onSubmit}
        title={isRunning ? "Stop turn" : "Send message"}
      >
        {isRunning ? (
          <Square size={15} fill="currentColor" />
        ) : isSending ? (
          <Loader2 size={17} className="spin" />
        ) : (
          <Send size={17} />
        )}
      </button>
    </div>
  );
}

const permissionModeOptions: Array<{
  value: AppSettings["permissionMode"];
  label: string;
}> = [
  { value: "chat", label: "仅聊天" },
  { value: "read_only", label: "只读" },
  { value: "auto", label: "自动" },
  { value: "approve", label: "需要审批" },
  { value: "full_access", label: "完全访问" },
];

function permissionModeLabel(mode: AppSettings["permissionMode"]): string {
  switch (mode) {
    case "full_access":
      return "完全访问";
    case "read_only":
      return "只读";
    case "approve":
      return "需要审批";
    case "chat":
      return "仅聊天";
    default:
      return "自动";
  }
}

function RightPanel({
  toolTabs,
  activeToolTab,
  conversationCollapsed,
  thread,
  workspaceRoot,
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
}: {
  toolTabs: ToolTab[];
  activeToolTab: ToolTab | null;
  conversationCollapsed: boolean;
  thread: Thread | null;
  workspaceRoot: string | null;
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
}) {
  const changedFiles = workspaceDiff?.files.length ?? 0;
  const enabledMcpServers = threadMcpServers.filter(
    (server) => server.enabled,
  ).length;

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
      <aside className="right-panel tool-stage">
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
            <UnavailableToolState
              icon={Globe2}
              title="内置浏览器尚未实现"
              description="录屏中的网页标签需要独立的 Electron BrowserView 安全边界、导航策略和会话隔离。入口已经保留，当前不会伪造网页执行结果。"
            />
          ) : (
            renderWorkbench("stage", activeToolTab.kind)
          )}
        </div>
      </aside>
    );
  }

  return (
    <aside className="right-panel">
      <section className="environment-panel" aria-label="环境信息">
        <header>
          <span>环境信息</span>
          <button disabled title="添加环境 · 未实现" aria-label="添加环境">
            <Plus size={14} />
          </button>
        </header>
        <div className="environment-facts">
          <button type="button" onClick={() => onOpenToolTab("diff")}>
            <FileCode2 size={14} />
            <span>变更</span>
            <strong>{changedFiles}</strong>
          </button>
          <button type="button" onClick={() => onOpenToolTab("terminal")}>
            <TerminalSquare size={14} />
            <span>本地</span>
            <strong>{terminalSession ? "已连接" : "待连接"}</strong>
          </button>
          <button type="button" onClick={() => onOpenToolTab("extensions")}>
            <Plug size={14} />
            <span>MCP</span>
            <strong>
              {enabledMcpServers}/{mcpServers.length}
            </strong>
          </button>
          <button type="button" onClick={() => onOpenToolTab("files")}>
            <GitBranch size={14} />
            <span>工作区</span>
            <strong title={workspaceRoot ?? ""}>
              {workspaceRoot ? workspaceName(workspaceRoot) : "未选择"}
            </strong>
          </button>
        </div>
        <div className="environment-disabled-actions">
          <button disabled title="提交或推送 · 未实现">
            <GitBranch size={14} />
            <span>提交或推送</span>
            <small>未实现</small>
          </button>
          <button disabled title="GitHub CLI · 未实现">
            <Github size={14} />
            <span>GitHub CLI</span>
            <small>未实现</small>
          </button>
          <button disabled title="比较分支 · 未实现">
            <GitPullRequest size={14} />
            <span>比较分支</span>
            <small>未实现</small>
          </button>
          <button disabled title="子智能体 · 未实现">
            <Users size={14} />
            <span>子智能体</span>
            <small>未实现</small>
          </button>
        </div>
      </section>
      {renderWorkbench("panel")}
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
                  {kind === "browser" && <small>未实现</small>}
                </button>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}

function UnavailableToolState({
  icon: Icon,
  title,
  description,
}: {
  icon: typeof Folder;
  title: string;
  description: string;
}) {
  return (
    <div className="unavailable-tool-state">
      <Icon size={34} />
      <h2>{title}</h2>
      <p>{description}</p>
      <span>未实现</span>
    </div>
  );
}

function NewTaskState({
  value,
  workspaceRoot,
  projectName,
  recentWorkspaces,
  model,
  permissionMode,
  isSending,
  onChange,
  onPickWorkspace,
  onSelectWorkspace,
  onOpenTool,
  onChangePermissionMode,
  onSubmit,
}: {
  value: string;
  workspaceRoot: string | null;
  projectName: string | null;
  recentWorkspaces: RecentWorkspace[];
  model: string;
  permissionMode: AppSettings["permissionMode"];
  isSending: boolean;
  onChange(value: string): void;
  onPickWorkspace(): void;
  onSelectWorkspace(workspaceRoot: string): void;
  onOpenTool(kind: ToolTabKind): void;
  onChangePermissionMode(mode: AppSettings["permissionMode"]): void;
  onSubmit(): void;
}) {
  const suggestions = [
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
    { icon: Activity, label: "修复问题", prompt: "检查并修复当前项目中的问题" },
  ];

  return (
    <>
      <div className="new-task-state">
        <Bot size={34} />
        <h2>
          我们应该在{" "}
          <u>
            {projectName ??
              (workspaceRoot ? workspaceName(workspaceRoot) : "项目")}
          </u>{" "}
          中构建什么？
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
        model={model}
        permissionMode={permissionMode}
        workspaceRoot={workspaceRoot}
        projectName={
          projectName ?? (workspaceRoot ? workspaceName(workspaceRoot) : null)
        }
        recentWorkspaces={recentWorkspaces}
        onChange={onChange}
        onSubmit={onSubmit}
        onCancel={() => undefined}
        onOpenTool={onOpenTool}
        onPickWorkspace={onPickWorkspace}
        onSelectWorkspace={onSelectWorkspace}
        onChangePermissionMode={onChangePermissionMode}
      />
    </>
  );
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

function ArtifactPreviewModal({
  preview,
  onOpenPath,
  onClose,
}: {
  preview: ArtifactPreviewState;
  onOpenPath(targetPath: string): void;
  onClose(): void;
}) {
  const content = preview.status === "ready" ? preview.content : null;

  return (
    <div className="modal-backdrop" role="presentation" onClick={onClose}>
      <section
        className="artifact-preview-modal"
        role="dialog"
        aria-modal="true"
        onClick={(event) => event.stopPropagation()}
      >
        <header>
          <div>
            <FileCode2 size={17} />
            <strong>Artifact</strong>
            <span title={preview.artifactId}>{preview.artifactId}</span>
          </div>
          <button
            className="secondary-button compact"
            type="button"
            onClick={onClose}
          >
            Close
          </button>
        </header>
        {preview.status === "loading" && (
          <div className="workbench-empty-state">Loading artifact...</div>
        )}
        {preview.status === "error" && (
          <p className="message-error">{preview.message}</p>
        )}
        {content && (
          <>
            <div className="artifact-preview-editor">
              <MonacoEditor
                value={content.content}
                language={detectLanguage(
                  content.filePath ?? preview.artifactId,
                )}
                readOnly
              />
            </div>
            {content.filePath && (
              <button
                className="artifact-file-link"
                type="button"
                title={content.filePath}
                onClick={() => onOpenPath(content.filePath ?? "")}
              >
                <ExternalLink size={12} />
                {content.filePath}
              </button>
            )}
          </>
        )}
      </section>
    </div>
  );
}

type ArtifactReference = {
  id: string;
  kind?: string;
  bytes?: number;
};

type LocalProject = {
  id: string;
  name: string;
};

const localProjectsStorageKey = "opentopia.localProjects";

function readLocalProjects(): LocalProject[] {
  try {
    const parsed = JSON.parse(
      window.localStorage.getItem(localProjectsStorageKey) ?? "[]",
    );
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((project): project is LocalProject =>
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

function writeLocalProjects(projects: LocalProject[]) {
  window.localStorage.setItem(
    localProjectsStorageKey,
    JSON.stringify(projects),
  );
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
  }
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
