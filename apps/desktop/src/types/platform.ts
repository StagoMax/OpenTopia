import type { KeyringMetadata, LogFileInfo, SecretSources } from "./provider";
import type {
  BrowserProfilePersistence,
  ChromeBridgeState,
  WebPreviewBounds,
  WebPreviewState,
} from "./workspace";

export type PlatformInfo = {
  platform: "desktop" | "web";
  os?: string;
  arch?: string;
  versions?: Record<string, string>;
  backendUrl: string;
  apiToken: string;
  keyring?: KeyringMetadata;
  paths?: {
    userData?: string | null;
    logs?: string | null;
    crashLogs?: string | null;
  };
  protocol?: {
    scheme: string;
    registered: boolean;
  };
};

export type DesktopToolMenuAction =
  "flow" | "terminal" | "browser" | "computer" | "files" | "side-task";

export type DesktopToolMenuRequest = {
  canOpenFlow: boolean;
  canOpenSideTask: boolean;
  x?: number;
  y?: number;
};

export type BackendEventStreamMessage = {
  streamId: string;
  type: "connected" | "chunk" | "error" | "closed";
  status?: number;
  chunk?: string;
  error?: string;
  reason?: string;
};

/**
 * Startup activity reported by the desktop main process while its managed
 * local backend is being prepared. A build has no reliable total work count,
 * so consumers should present this as indeterminate progress.
 */
export type BackendStartupPhase =
  | "checking"
  | "compiling"
  | "starting"
  | "waiting_for_health"
  | "ready"
  | "failed";

export type BackendStartupStatus = {
  phase: BackendStartupPhase;
  /** The active Cargo package when compilation is in progress. */
  detail: string | null;
  startedAt: string;
  updatedAt: string;
};

export type ManagedPowerShellStatus =
  "not_required" | "pending" | "downloading" | "ready" | "disabled" | "failed";

export type ShellRuntimeStatus = {
  runtime: {
    program: string;
    dialect: "power_shell7" | "windows_power_shell51" | "posix_sh";
    version: string | null;
    source: "configured" | "managed" | "standard_install" | "path" | "system";
  };
  managedVersion: string;
  managedStatus: ManagedPowerShellStatus;
  managedError?: string;
};

export type ManagedOfficeRuntimeStatus =
  "not_required" | "pending" | "downloading" | "ready" | "disabled" | "failed";

export type OfficeRuntimeStatus = {
  runtime?: {
    executable: string;
    root: string;
    runtimeVersion: string;
    pythonVersion: string;
    openpyxlVersion: string;
    source: "configured" | "packaged" | "managed" | "legacy_override";
  };
  managedVersion: string;
  managedStatus: ManagedOfficeRuntimeStatus;
  managedError?: string;
};

export type BackendHealth = {
  ok: boolean;
  service: string;
  apiVersion: number;
  officeRuntime: OfficeRuntimeStatus;
  shellRuntime: ShellRuntimeStatus;
};

export type LibraryProviderId = "sag" | "graph-rag";

export type LibraryProviderServiceRuntimeStatus = {
  provider?: LibraryProviderId;
  state: "ready" | "unavailable";
  endpoint: string;
  managed: boolean;
  canStart: boolean;
  source: string | null;
  message?: string;
};

export type SagServiceRuntimeStatus = LibraryProviderServiceRuntimeStatus;

export type SystemNotificationOptions = {
  title: string;
  body: string;
  silent?: boolean;
};

export type RecentWorkspace = {
  workspaceRoot: string;
  name: string;
  lastOpenedAt: string;
};

export type WorkspacePickResult =
  | { canceled: true }
  | {
      canceled: false;
      workspaceRoot: string;
      workspace: RecentWorkspace;
      recentWorkspaces: RecentWorkspace[];
    };

export type ContextSourceFile = {
  path: string;
  name: string;
  extension: string;
  kind: "text" | "image" | "document";
  bytes: number;
};

export type ContextSourcePickResult =
  | { canceled: true; files: [] }
  | { canceled: false; files: ContextSourceFile[] };

export type InlineImageAttachment = {
  id: string;
  contentType: string;
  data: number[];
  name?: string;
};

export type InlineMessageContentPart =
  | { type: "text"; text: string }
  | { type: "image_ref"; imageId: string }
  | { type: "attachment_ref"; path: string; name: string };

export type PluginDirectoryPickResult =
  { canceled: true } | { canceled: false; path: string };

export type BrowserContent =
  | { type: "text"; text: string; truncated: boolean }
  | { type: "json"; value: unknown }
  | { type: "image"; mime_type: string; bytes: number[] }
  | {
      type: "file";
      path: string;
      mime_type?: string | null;
      bytes: number;
    };

export type BrowserOutput = {
  url?: string | null;
  contents: BrowserContent[];
  metadata: unknown;
};

export type BrowserRect = {
  x: number;
  y: number;
  width: number;
  height: number;
};

export type BrowserNode = {
  nodeRef: string;
  role: string;
  name: string;
  tagName: string;
  bounds: BrowserRect;
  targetRef?: string | null;
  frameRef?: string | null;
  href?: string | null;
  formAction?: string | null;
  editable: boolean;
};

export type BrowserTarget = {
  targetRef: string;
  url: string;
  title: string;
  active: boolean;
  opener?: string | null;
};

export type BrowserFrame = {
  frameRef: string;
  targetRef: string;
  parentFrameRef?: string | null;
  url: string;
  name: string;
};

export type BrowserAccessibilityNode = {
  axNodeId: string;
  parentAxNodeId?: string | null;
  role: string;
  name: string;
  value?: string | null;
  description?: string | null;
  ignored: boolean;
  targetRef: string;
  frameRef?: string | null;
  nodeRef?: string | null;
};

export type BrowserDialog = {
  dialogType: string;
  message: string;
  defaultPrompt?: string | null;
  handled: boolean;
  targetRef: string;
};

export type BrowserObservation = {
  observationId: string;
  url: string;
  title: string;
  text: string;
  textTruncated: boolean;
  nodes: BrowserNode[];
  targets: BrowserTarget[];
  frames: BrowserFrame[];
  accessibilityTree: BrowserAccessibilityNode[];
  dialogs: BrowserDialog[];
};

export type ScreenRect = {
  x: number;
  y: number;
  width: number;
  height: number;
};

export type ComputerWindowTarget = {
  windowId: string;
  processId: number;
  title: string;
  executable?: string | null;
  bounds: ScreenRect;
  isForeground: boolean;
};

export type ComputerScreenshot = {
  mimeType: string;
  bytes: number[];
};

export type ComputerObservation = {
  observationId: string;
  sessionId: string;
  target: ComputerWindowTarget;
  captureRect: ScreenRect;
  imageWidth: number;
  imageHeight: number;
  screenshot?: ComputerScreenshot | null;
  accessibilityTree?: unknown | null;
  unstable: boolean;
  capturedAt: string;
};

export type PlatformOpenRequest = {
  id: string;
  source: string;
  kind: string;
  action?: string;
  threadId?: string;
  workspaceRoot?: string;
  path?: string;
  receivedAt: string;
};

export type FileLinkAction =
  "open-default" | "open-vscode" | "open-with" | "save-as" | "reveal";

export type FileLinkActionRequest = {
  action: FileLinkAction;
  path: string;
  line?: number | null;
};

export type FileLinkActionResult = {
  canceled?: boolean;
  path?: string;
};

declare global {
  interface Window {
    opentopia?: {
      newWindow(): Promise<boolean>;
      closeWindow(): Promise<boolean>;
      quit(): Promise<boolean>;
      getPlatformInfo(): Promise<PlatformInfo>;
      showToolMenu(
        options: DesktopToolMenuRequest,
      ): Promise<DesktopToolMenuAction | null>;
      getBackendStartupStatus(): Promise<BackendStartupStatus>;
      onBackendStartupStatus(
        listener: (status: BackendStartupStatus) => void,
      ): () => void;
      ensureSagLibraryService(): Promise<SagServiceRuntimeStatus>;
      ensureLibraryProviderService(
        provider: LibraryProviderId,
      ): Promise<LibraryProviderServiceRuntimeStatus>;
      getOpenRequests(): Promise<PlatformOpenRequest[]>;
      onOpenRequest(
        listener: (request: PlatformOpenRequest) => void,
      ): () => void;
      /** Repaints the OS-drawn window chrome for the resolved theme. */
      setTheme(theme: "light" | "dark"): Promise<boolean>;
      openExternal(url: string): Promise<void>;
      openPath(targetPath: string): Promise<{ path: string }>;
      performFileLinkAction(
        request: FileLinkActionRequest,
      ): Promise<FileLinkActionResult>;
      showSystemNotification(
        options: SystemNotificationOptions,
      ): Promise<boolean>;
      writeClipboardImage(bytes: Uint8Array): Promise<boolean>;
      selectWorkspace(options?: {
        defaultPath?: string;
      }): Promise<WorkspacePickResult>;
      selectContextFiles(options?: {
        defaultPath?: string;
      }): Promise<ContextSourcePickResult>;
      getDroppedContextFiles(files: File[]): Promise<ContextSourcePickResult>;
      selectPluginDirectory(options?: {
        defaultPath?: string;
      }): Promise<PluginDirectoryPickResult>;
      getRecentWorkspaces(): Promise<RecentWorkspace[]>;
      saveRecentWorkspace(workspaceRoot: string): Promise<RecentWorkspace[]>;
      removeRecentWorkspace(workspaceRoot: string): Promise<RecentWorkspace[]>;
      clearRecentWorkspaces(): Promise<RecentWorkspace[]>;
      listSecretSources(): Promise<SecretSources>;
      setSecret(key: string, value: string): Promise<void>;
      deleteSecret(key: string): Promise<void>;
      getProviderApiKeyMetadata(providerId: string): Promise<KeyringMetadata>;
      setProviderApiKey(
        providerId: string,
        value: string,
      ): Promise<KeyringMetadata>;
      deleteProviderApiKey(providerId: string): Promise<KeyringMetadata>;
      listLogFiles(): Promise<LogFileInfo[]>;
      readLogFile(
        path: string,
        offset?: number,
        limit?: number,
      ): Promise<{ lines: string[]; total: number }>;
      recordConversationRenderTrace?(trace: {
        stage: "received" | "committed" | "painted";
        channel: "assistant" | "commentary" | "reasoning" | "status";
        threadId: string;
        turnId?: string | null;
        eventId?: string;
        messageId?: string;
        seq?: number;
        sourceCreatedAt?: string;
        rendererAt: string;
        rendererClockMs: number;
        latencyMs?: number;
        change: "append" | "replace";
        text: string;
        textLength: number;
        visible: boolean;
      }): void;
      recordConversationSendTrace?(trace: {
        stage:
          | "controller_started"
          | "state_dispatched"
          | "fetch_started"
          | "response_headers"
          | "response_parsed"
          | "state_confirmed"
          | "failed";
        requestId: string;
        threadId: string;
        rendererAt: string;
        rendererClockMs: number;
        elapsedMs: number;
        clientStartedAtMs: number;
        turnId?: string | null;
        messageId?: string;
        queued?: boolean;
        httpStatus?: number;
        serverDurationMs?: number;
        clientToServerMs?: number;
        errorName?: string;
      }): void;
      openBackendEventStream?(streamId: string, path: string): void;
      closeBackendEventStream?(streamId: string): void;
      onBackendEventStreamMessage?(
        listener: (message: BackendEventStreamMessage) => void,
      ): () => void;
      browserHost?: {
        createSession(input: {
          sessionId: string;
          profileId?: string;
          profilePersistence?: BrowserProfilePersistence;
          url?: string;
          bounds?: WebPreviewBounds;
          visible?: boolean;
        }): Promise<WebPreviewState>;
        destroySession(sessionId: string): Promise<void>;
        getState(sessionId: string): Promise<WebPreviewState>;
        navigate(sessionId: string, url: string): Promise<unknown>;
        navigateFromAddressBar(
          sessionId: string,
          url: string,
        ): Promise<unknown>;
        beginUserControl(sessionId: string): Promise<unknown>;
        back(sessionId: string): Promise<WebPreviewState>;
        forward(sessionId: string): Promise<WebPreviewState>;
        reload(sessionId: string): Promise<WebPreviewState>;
        setBounds(
          sessionId: string,
          bounds: WebPreviewBounds,
        ): Promise<unknown>;
        setVisibility(sessionId: string, visible: boolean): Promise<unknown>;
        show(sessionId: string, bounds?: WebPreviewBounds): Promise<unknown>;
        hide(sessionId: string): Promise<unknown>;
        onStateChanged(
          listener: (state: WebPreviewState) => void,
        ): (() => void) | void;
      };
      chromeBridge?: {
        startPairing(sessionId: string): Promise<ChromeBridgeState>;
        getStatus(sessionId: string): Promise<ChromeBridgeState>;
        disconnect(sessionId: string): Promise<ChromeBridgeState>;
        runAction(
          sessionId: string,
          action: "navigate" | "back" | "forward" | "reload",
          value?: string,
        ): Promise<ChromeBridgeState>;
        onStateChanged(
          listener: (state: ChromeBridgeState) => void,
        ): (() => void) | void;
      };
    };
  }
}
