import type {
  LogFileInfo,
  PlatformInfo,
  RecentWorkspace,
  SecretSources,
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

export function getLoadedApiToken(): string {
  if (!loadedApiToken) {
    throw new Error("OpenTopia API credentials have not been initialized");
  }
  return loadedApiToken;
}

export async function selectWorkspace(options?: {
  defaultPath?: string;
}): Promise<WorkspacePickResult> {
  if (window.opentopia) return window.opentopia.selectWorkspace(options);
  return { canceled: true };
}

export async function openPath(targetPath: string): Promise<void> {
  if (!window.opentopia) return;
  await window.opentopia.openPath(targetPath);
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
