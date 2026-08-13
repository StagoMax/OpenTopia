const { contextBridge, ipcRenderer, webUtils } = require("electron");

const browserHost = Object.freeze({
  createSession: (options) =>
    ipcRenderer.invoke("browser-host:create", options),
  destroySession: (sessionId) =>
    ipcRenderer.invoke("browser-host:destroy", sessionId),
  getState: (sessionId) =>
    ipcRenderer.invoke("browser-host:get-state", sessionId),
  navigate: (sessionId, url) =>
    ipcRenderer.invoke("browser-host:navigate", sessionId, url),
  navigateFromAddressBar: (sessionId, url) =>
    ipcRenderer.invoke(
      "browser-host:navigate-from-address-bar",
      sessionId,
      url,
    ),
  beginUserControl: (sessionId) =>
    ipcRenderer.invoke("browser-host:begin-user-control", sessionId),
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

const chromeBridge = Object.freeze({
  startPairing: (sessionId) =>
    ipcRenderer.invoke("chrome-bridge:start-pairing", sessionId),
  getStatus: (sessionId) =>
    ipcRenderer.invoke("chrome-bridge:get-status", sessionId),
  disconnect: (sessionId) =>
    ipcRenderer.invoke("chrome-bridge:disconnect", sessionId),
  onStateChanged: (listener) => {
    if (typeof listener !== "function") {
      throw new TypeError("Chrome bridge state listener must be a function.");
    }
    const wrapped = (_event, state) => listener(state);
    ipcRenderer.on("chrome-bridge:state", wrapped);
    return () => ipcRenderer.removeListener("chrome-bridge:state", wrapped);
  },
});

contextBridge.exposeInMainWorld("opentopia", {
  newWindow: () => ipcRenderer.invoke("platform:new-window"),
  closeWindow: () => ipcRenderer.invoke("platform:close-window"),
  quit: () => ipcRenderer.invoke("platform:quit"),
  getPlatformInfo: () => ipcRenderer.invoke("platform:get-info"),
  ensureSagLibraryService: () => ipcRenderer.invoke("library:sag:ensure-ready"),
  getOpenRequests: () => ipcRenderer.invoke("platform:get-open-requests"),
  onOpenRequest: (listener) => {
    if (typeof listener !== "function") {
      throw new TypeError("Open request listener must be a function.");
    }
    const wrapped = (_event, request) => listener(request);
    ipcRenderer.on("platform:open-request", wrapped);
    return () => ipcRenderer.removeListener("platform:open-request", wrapped);
  },
  setTheme: (theme) => ipcRenderer.invoke("platform:set-theme", theme),
  openExternal: (url) => ipcRenderer.invoke("platform:open-external", url),
  openPath: (targetPath) =>
    ipcRenderer.invoke("platform:open-path", targetPath),
  performFileLinkAction: (request) =>
    ipcRenderer.invoke("platform:file-link-action", request),
  showSystemNotification: (options) =>
    ipcRenderer.invoke("platform:show-system-notification", options),
  writeClipboardImage: (bytes) =>
    ipcRenderer.invoke("platform:write-clipboard-image", bytes),
  selectWorkspace: (options) => ipcRenderer.invoke("workspace:select", options),
  selectContextFiles: (options) =>
    ipcRenderer.invoke("context:select-files", options),
  getDroppedContextFiles: (files) =>
    ipcRenderer.invoke(
      "context:add-dropped-files",
      Array.from(files, (file) => webUtils.getPathForFile(file)).filter(
        Boolean,
      ),
    ),
  selectPluginDirectory: (options) =>
    ipcRenderer.invoke("plugins:select-directory", options),
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
  recordConversationRenderTrace: (trace) =>
    ipcRenderer.send("logs:conversation-render-trace", trace),
  browserHost,
  chromeBridge,
});
