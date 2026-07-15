const { contextBridge, ipcRenderer } = require("electron");

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
  listLogFiles: () => ipcRenderer.invoke("logs:list"),
  readLogFile: (path, offset, limit) =>
    ipcRenderer.invoke("logs:read", path, offset, limit),
});
