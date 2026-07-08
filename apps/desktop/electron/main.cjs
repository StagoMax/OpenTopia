const { app, BrowserWindow, ipcMain, shell } = require("electron")
const path = require("node:path")
const { URL } = require("node:url")

const isDev = !app.isPackaged
const defaultBackendUrl = process.env.OPENTOPIA_SERVER_URL || "http://127.0.0.1:8787"

let mainWindow = null

function createMainWindow() {
  mainWindow = new BrowserWindow({
    width: 1320,
    height: 860,
    minWidth: 1080,
    minHeight: 720,
    title: "OpenTopia",
    backgroundColor: "#0f1115",
    show: false,
    webPreferences: {
      preload: path.join(__dirname, "preload.cjs"),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: false,
    },
  })

  mainWindow.once("ready-to-show", () => {
    mainWindow.show()
  })

  if (isDev) {
    mainWindow.loadURL(process.env.VITE_DEV_SERVER_URL || "http://127.0.0.1:5173")
    mainWindow.webContents.openDevTools({ mode: "detach" })
  } else {
    mainWindow.loadFile(path.join(__dirname, "..", "dist", "index.html"))
  }
}

function registerIpc() {
  ipcMain.handle("platform:get-info", () => ({
    platform: "desktop",
    os: process.platform,
    arch: process.arch,
    versions: process.versions,
    backendUrl: defaultBackendUrl,
  }))

  ipcMain.handle("platform:open-external", async (_event, rawUrl) => {
    const url = new URL(rawUrl)
    if (!["http:", "https:", "mailto:"].includes(url.protocol)) {
      throw new Error(`Blocked external URL protocol: ${url.protocol}`)
    }
    await shell.openExternal(url.toString())
  })
}

const singleInstance = app.requestSingleInstanceLock()
if (!singleInstance) {
  app.quit()
} else {
  app.on("second-instance", () => {
    if (!mainWindow) return
    if (mainWindow.isMinimized()) mainWindow.restore()
    mainWindow.focus()
  })

  app.whenReady().then(() => {
    registerIpc()
    createMainWindow()

    app.on("activate", () => {
      if (BrowserWindow.getAllWindows().length === 0) createMainWindow()
    })
  })
}

app.on("window-all-closed", () => {
  if (process.platform !== "darwin") app.quit()
})
