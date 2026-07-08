const { contextBridge, ipcRenderer } = require("electron")

contextBridge.exposeInMainWorld("opentopia", {
  getPlatformInfo: () => ipcRenderer.invoke("platform:get-info"),
  openExternal: (url) => ipcRenderer.invoke("platform:open-external", url),
})
