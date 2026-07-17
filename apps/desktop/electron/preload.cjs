const { contextBridge, ipcRenderer } = require("electron");

const browserHost = Object.freeze({
  createSession: (options) =>
    ipcRenderer.invoke("browser-host:create", options),
  destroySession: (sessionId) =>
    ipcRenderer.invoke("browser-host:destroy", sessionId),
  getState: (sessionId) =>
    ipcRenderer.invoke("browser-host:get-state", sessionId),
  navigate: (sessionId, url) =>
    ipcRenderer.invoke("browser-host:navigate", sessionId, url),
  back: (sessionId) => ipcRenderer.invoke("browser-host:back", sessionId),
  forward: (sessionId) => ipcRenderer.invoke("browser-host:forward", sessionId),
  reload: (sessionId) => ipcRenderer.invoke("browser-host:reload", sessionId),
  setBounds: (sessionId, bounds) =>
    ipcRenderer.invoke("browser-host:set-bounds", sessionId, bounds),
  setVisibility: (sessionId, visible) =>
    ipcRenderer.invoke("browser-host:set-visibility", sessionId, visible),
  show: (sessionId, bounds) =>
    ipcRenderer.invoke("browser-host:show", sessionId, bounds),
  hide: (sessionId) => ipcRenderer.invoke("browser-host:hide", sessionId),
  onStateChanged: (listener) => {
    if (typeof listener !== "function") {
      throw new TypeError("Browser state listener must be a function.");
    }
    const wrapped = (_event, state) => listener(state);
    ipcRenderer.on("browser-host:state", wrapped);
    return () => ipcRenderer.removeListener("browser-host:state", wrapped);
  },
});

contextBridge.exposeInMainWorld("opentopia", {
  getPlatformInfo: () => ipcRenderer.invoke("platform:get-info"),
  openExternal: (url) => ipcRenderer.invoke("platform:open-external", url),
  openPath: (targetPath) =>
    ipcRenderer.invoke("platform:open-path", targetPath),
  selectWorkspace: (options) => ipcRenderer.invoke("workspace:select", options),
  selectContextFiles: (options) =>
    ipcRenderer.invoke("context:select-files", options),
  getRecentWorkspaces: () => ipcRenderer.invoke("workspace:get-recent"),
  saveRecentWorkspace: (workspaceRoot) =>
    ipcRenderer.invoke("workspace:save-recent", workspaceRoot),
  removeRecentWorkspace: (workspaceRoot) =>
    ipcRenderer.invoke("workspace:remove-recent", workspaceRoot),
  clearRecentWorkspaces: () => ipcRenderer.invoke("workspace:clear-recent"),
  listSecretSources: () => ipcRenderer.invoke("secrets:list-sources"),
  setSecret: (key, value) => ipcRenderer.invoke("secrets:set", key, value),
  deleteSecret: (key) => ipcRenderer.invoke("secrets:delete", key),
  getProviderApiKeyMetadata: (providerId) =>
    ipcRenderer.invoke("secrets:get-provider-key-metadata", providerId),
  setProviderApiKey: (providerId, value) =>
    ipcRenderer.invoke("secrets:set-provider-key", providerId, value),
  deleteProviderApiKey: (providerId) =>
    ipcRenderer.invoke("secrets:delete-provider-key", providerId),
  listLogFiles: () => ipcRenderer.invoke("logs:list"),
  readLogFile: (path, offset, limit) =>
    ipcRenderer.invoke("logs:read", path, offset, limit),
  browserHost,
});
