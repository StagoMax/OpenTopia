import type { ConversationRenderTrace } from "./conversationRenderTrace";
import type { ConversationSendTrace } from "./conversationSendTrace";
import type { BackendStartupStatus } from "./types/platform";
import type {
  ContextSourcePickResult,
  FileLinkActionRequest,
  FileLinkActionResult,
  KeyringMetadata,
  LogFileInfo,
  LibraryProviderId,
  LibraryProviderServiceRuntimeStatus,
  PlatformInfo,
  PluginDirectoryPickResult,
  RecentWorkspace,
  SagServiceRuntimeStatus,
  SecretSources,
  SystemNotificationOptions,
  WorkspacePickResult,
} from "./types";

const browserRecentWorkspacesKey = "opentopia.recentWorkspaces";
const maxRecentWorkspaces = 12;
let loadedApiToken: string | null = null;
const unavailableKeyring = {
  available: false,
  encryptionAvailable: false,
  storageBackend: null,
  providerApiKeyConfigured: false,
  providerApiKeySourceId: "keyring:provider-api-key",
  envTarget: "OPENTOPIA_API_KEY",
  status: "unavailable",
};
export async function loadPlatformInfo(): Promise<PlatformInfo> {
  const info = window.opentopia
    ? await window.opentopia.getPlatformInfo()
    : {
        platform: "web",
        backendUrl:
          import.meta.env.VITE_OPENTOPIA_SERVER_URL || "http://127.0.0.1:8787",
        apiToken: import.meta.env.VITE_OPENTOPIA_API_TOKEN || "",
        keyring: unavailableKeyring,
      };
  loadedApiToken = info.apiToken;
  return info as PlatformInfo;
}

export async function getBackendStartupStatus(): Promise<BackendStartupStatus | null> {
  if (!window.opentopia?.getBackendStartupStatus) return null;
  return window.opentopia.getBackendStartupStatus();
}

export function onBackendStartupStatus(
  listener: (status: BackendStartupStatus) => void,
): () => void {
  return window.opentopia?.onBackendStartupStatus?.(listener) ?? (() => {});
}

export async function ensureSagLibraryService(): Promise<SagServiceRuntimeStatus | null> {
  if (!window.opentopia?.ensureSagLibraryService) return null;
  return window.opentopia.ensureSagLibraryService();
}

export async function ensureLibraryProviderService(
  provider: LibraryProviderId,
): Promise<LibraryProviderServiceRuntimeStatus | null> {
  if (window.opentopia?.ensureLibraryProviderService) {
    return window.opentopia.ensureLibraryProviderService(provider);
  }
  if (provider === "sag") return ensureSagLibraryService();
  return null;
}

export function getLoadedApiToken(): string {
  if (!loadedApiToken) {
    throw new Error("OpenTopia API credentials have not been initialized");
  }
  return loadedApiToken;
}

export async function newAppWindow(): Promise<void> {
  if (window.opentopia) {
    await window.opentopia.newWindow();
    return;
  }
  window.open(window.location.href, "_blank", "noopener,noreferrer");
}

export async function closeAppWindow(): Promise<void> {
  if (window.opentopia) {
    await window.opentopia.closeWindow();
    return;
  }
  window.close();
}

export async function quitApp(): Promise<void> {
  if (window.opentopia) {
    await window.opentopia.quit();
    return;
  }
  window.close();
}

export async function selectWorkspace(options?: {
  defaultPath?: string;
}): Promise<WorkspacePickResult> {
  if (window.opentopia) return window.opentopia.selectWorkspace(options);
  return { canceled: true };
}

export async function selectContextFiles(options?: {
  defaultPath?: string;
}): Promise<ContextSourcePickResult> {
  if (window.opentopia) return window.opentopia.selectContextFiles(options);
  return { canceled: true, files: [] };
}

export async function getDroppedContextFiles(
  files: File[],
): Promise<ContextSourcePickResult> {
  if (window.opentopia) return window.opentopia.getDroppedContextFiles(files);
  return { canceled: true, files: [] };
}

export async function selectPluginDirectory(options?: {
  defaultPath?: string;
}): Promise<PluginDirectoryPickResult> {
  if (window.opentopia) return window.opentopia.selectPluginDirectory(options);
  return { canceled: true };
}

export async function openPath(targetPath: string): Promise<void> {
  if (!window.opentopia) return;
  await window.opentopia.openPath(targetPath);
}

export async function performFileLinkAction(
  request: FileLinkActionRequest,
): Promise<FileLinkActionResult> {
  if (!window.opentopia?.performFileLinkAction) {
    throw new Error("文件操作仅在桌面应用中可用。");
  }
  return window.opentopia.performFileLinkAction(request);
}

export async function openExternal(url: string): Promise<void> {
  if (window.opentopia) {
    await window.opentopia.openExternal(url);
    return;
  }
  window.open(url, "_blank", "noopener,noreferrer");
}

export async function showSystemNotification(
  options: SystemNotificationOptions,
): Promise<boolean> {
  if (window.opentopia?.showSystemNotification) {
    return window.opentopia.showSystemNotification(options);
  }

  if (!("Notification" in window)) return false;
  if (Notification.permission === "default") {
    const permission = await Notification.requestPermission();
    if (permission !== "granted") return false;
  } else if (Notification.permission !== "granted") {
    return false;
  }

  new Notification(options.title, {
    body: options.body,
    silent: options.silent,
  });
  return true;
}

export async function writeClipboardImage(pngBlob: Blob): Promise<void> {
  if (window.opentopia?.writeClipboardImage) {
    const bytes = new Uint8Array(await pngBlob.arrayBuffer());
    await window.opentopia.writeClipboardImage(bytes);
    return;
  }
  if (!navigator.clipboard?.write || typeof ClipboardItem === "undefined") {
    throw new Error("当前环境不支持复制图片");
  }
  await navigator.clipboard.write([
    new ClipboardItem({ [pngBlob.type]: pngBlob }),
  ]);
}

export async function getRecentWorkspaces(): Promise<RecentWorkspace[]> {
  if (window.opentopia) return window.opentopia.getRecentWorkspaces();
  return readBrowserRecentWorkspaces();
}

export async function saveRecentWorkspace(
  workspaceRoot: string,
): Promise<RecentWorkspace[]> {
  if (window.opentopia) {
    return window.opentopia.saveRecentWorkspace(workspaceRoot);
  }

  const key = workspaceKey(workspaceRoot);
  const next = [
    toRecentWorkspace(workspaceRoot),
    ...readBrowserRecentWorkspaces().filter(
      (workspace) => workspaceKey(workspace.workspaceRoot) !== key,
    ),
  ].slice(0, maxRecentWorkspaces);
  writeBrowserRecentWorkspaces(next);
  return next;
}

export async function removeRecentWorkspace(
  workspaceRoot: string,
): Promise<RecentWorkspace[]> {
  if (window.opentopia) {
    return window.opentopia.removeRecentWorkspace(workspaceRoot);
  }

  const key = workspaceKey(workspaceRoot);
  const next = readBrowserRecentWorkspaces().filter(
    (workspace) => workspaceKey(workspace.workspaceRoot) !== key,
  );
  writeBrowserRecentWorkspaces(next);
  return next;
}

export async function setSecret(key: string, value: string): Promise<void> {
  if (window.opentopia?.setSecret) {
    return window.opentopia.setSecret(key, value);
  }
  throw new Error("Secret storage not available in web mode");
}

export async function listSecretSources(): Promise<SecretSources> {
  if (window.opentopia?.listSecretSources) {
    return window.opentopia.listSecretSources();
  }
  return {
    activeProviderKeySource: null,
    keyring: unavailableKeyring,
    sources: [],
    notes: ["Secret metadata is available only in the desktop app."],
  };
}

export async function deleteSecret(key: string): Promise<void> {
  if (window.opentopia?.deleteSecret) {
    return window.opentopia.deleteSecret(key);
  }
  throw new Error("Secret storage not available in web mode");
}

export async function getProviderApiKeyMetadata(
  providerId: string,
): Promise<KeyringMetadata> {
  if (window.opentopia?.getProviderApiKeyMetadata) {
    return window.opentopia.getProviderApiKeyMetadata(providerId);
  }
  throw new Error("Provider credential storage is not available in web mode");
}

export async function setProviderApiKey(
  providerId: string,
  value: string,
): Promise<KeyringMetadata> {
  if (window.opentopia?.setProviderApiKey) {
    return window.opentopia.setProviderApiKey(providerId, value);
  }
  throw new Error("Provider credential storage is not available in web mode");
}

export async function deleteProviderApiKey(
  providerId: string,
): Promise<KeyringMetadata> {
  if (window.opentopia?.deleteProviderApiKey) {
    return window.opentopia.deleteProviderApiKey(providerId);
  }
  throw new Error("Provider credential storage is not available in web mode");
}

export async function listLogFiles(): Promise<LogFileInfo[]> {
  if (window.opentopia?.listLogFiles) {
    return window.opentopia.listLogFiles();
  }
  return [];
}

export async function readLogFile(
  path: string,
  offset?: number,
  limit?: number,
): Promise<{ lines: string[]; total: number }> {
  if (window.opentopia?.readLogFile) {
    return window.opentopia.readLogFile(path, offset, limit);
  }
  return { lines: [], total: 0 };
}

export function recordConversationRenderTrace(
  trace: ConversationRenderTrace,
): void {
  window.opentopia?.recordConversationRenderTrace?.(trace);
}

export function recordConversationSendTrace(
  trace: ConversationSendTrace,
): void {
  if (typeof window === "undefined") return;
  window.opentopia?.recordConversationSendTrace?.(trace);
}

function readBrowserRecentWorkspaces(): RecentWorkspace[] {
  try {
    const parsed = JSON.parse(
      window.localStorage.getItem(browserRecentWorkspacesKey) || "[]",
    );
    if (!Array.isArray(parsed)) return [];
    return parsed
      .map((entry) => {
        if (!entry || typeof entry.workspaceRoot !== "string") return null;
        return {
          workspaceRoot: entry.workspaceRoot,
          name:
            typeof entry.name === "string"
              ? entry.name
              : workspaceName(entry.workspaceRoot),
          lastOpenedAt:
            typeof entry.lastOpenedAt === "string"
              ? entry.lastOpenedAt
              : new Date().toISOString(),
        };
      })
      .filter((entry): entry is RecentWorkspace => Boolean(entry));
  } catch {
    return [];
  }
}

function writeBrowserRecentWorkspaces(workspaces: RecentWorkspace[]) {
  window.localStorage.setItem(
    browserRecentWorkspacesKey,
    JSON.stringify(workspaces),
  );
}

function toRecentWorkspace(workspaceRoot: string): RecentWorkspace {
  return {
    workspaceRoot,
    name: workspaceName(workspaceRoot),
    lastOpenedAt: new Date().toISOString(),
  };
}

function workspaceName(workspaceRoot: string): string {
  const trimmed = workspaceRoot.replace(/[\\\/]+$/, "");
  const parts = trimmed.split(/[\\\/]/).filter(Boolean);
  return parts.at(-1) || workspaceRoot;
}

function workspaceKey(workspaceRoot: string): string {
  return workspaceRoot.toLocaleLowerCase();
}
