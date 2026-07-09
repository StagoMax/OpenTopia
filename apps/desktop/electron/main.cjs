const { app, BrowserWindow, ipcMain, shell } = require("electron")
const path = require("node:path")
const { URL } = require("node:url")
const { spawn } = require("node:child_process")
const fs = require("node:fs")

const isDev = !app.isPackaged
const defaultBackendUrl = process.env.OPENTOPIA_SERVER_URL || "http://127.0.0.1:8787"

let mainWindow = null
let backendProcess = null

function prependPath(env, entry) {
  if (!entry || !fs.existsSync(entry)) return

  const pathKey = Object.keys(env).find((key) => key.toLowerCase() === "path") || "PATH"
  const current = env[pathKey] || ""
  const entries = current.split(path.delimiter).filter(Boolean)
  const normalizedEntry = entry.toLowerCase()
  const alreadyPresent = entries.some((candidate) => candidate.toLowerCase() === normalizedEntry)
  if (!alreadyPresent) {
    env[pathKey] = [entry, ...entries].join(path.delimiter)
  }
}

function resolveMingwBin() {
  if (process.env.OPENTOPIA_MINGW_BIN && fs.existsSync(process.env.OPENTOPIA_MINGW_BIN)) {
    return process.env.OPENTOPIA_MINGW_BIN
  }

  const localAppData = process.env.LOCALAPPDATA
  const candidates = [
    localAppData
      ? path.join(
          localAppData,
          "Microsoft",
          "WinGet",
          "Packages",
          "BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe",
          "mingw64",
          "bin",
        )
      : null,
    "C:\\msys64\\ucrt64\\bin",
    "C:\\msys64\\mingw64\\bin",
  ].filter(Boolean)

  return candidates.find((candidate) => fs.existsSync(path.join(candidate, "gcc.exe"))) || null
}

function createBackendEnv(repoRoot) {
  const env = {
    ...process.env,
    OPENTOPIA_DB: process.env.OPENTOPIA_DB || path.join(repoRoot, ".opentopia", "opentopia.db"),
    OPENTOPIA_PERMISSION: process.env.OPENTOPIA_PERMISSION || "auto",
  }

  if (process.platform === "win32") {
    env.RUSTUP_TOOLCHAIN =
      process.env.OPENTOPIA_RUST_TOOLCHAIN || process.env.RUSTUP_TOOLCHAIN || "stable-x86_64-pc-windows-gnu"
    if (process.env.USERPROFILE) prependPath(env, path.join(process.env.USERPROFILE, ".cargo", "bin"))
    prependPath(env, resolveMingwBin())
  }

  return env
}

async function isBackendHealthy() {
  try {
    const response = await fetch(`${defaultBackendUrl}/health`, { signal: AbortSignal.timeout(1200) })
    return response.ok
  } catch {
    return false
  }
}

async function startBackendIfNeeded() {
  if (await isBackendHealthy()) return

  const repoRoot = path.resolve(__dirname, "..", "..", "..")
  const packagedServerExe = path.join(process.resourcesPath || "", "opentopia-server.exe")
  const packagedServerUnix = path.join(process.resourcesPath || "", "opentopia-server")
  const packagedServer = fs.existsSync(packagedServerExe) ? packagedServerExe : packagedServerUnix
  const command = !isDev && fs.existsSync(packagedServer) ? packagedServer : "cargo"
  const args = command === "cargo" ? ["run", "-p", "opentopia-server"] : []
  const cwd = command === "cargo" ? repoRoot : undefined

  try {
    backendProcess = spawn(command, args, {
      cwd,
      env: createBackendEnv(repoRoot),
      stdio: ["ignore", "pipe", "pipe"],
      windowsHide: true,
    })

    backendProcess.stdout?.on("data", (chunk) => console.log(`[opentopia-server] ${chunk}`))
    backendProcess.stderr?.on("data", (chunk) => console.error(`[opentopia-server] ${chunk}`))
    backendProcess.on("exit", (code) => {
      console.log(`[opentopia-server] exited with ${code}`)
      backendProcess = null
    })

    for (let i = 0; i < 30; i += 1) {
      await new Promise((resolve) => setTimeout(resolve, 500))
      if (await isBackendHealthy()) return
    }
  } catch (error) {
    console.error("[opentopia-server] failed to start", error)
  }
}

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

  app.whenReady().then(async () => {
    registerIpc()
    await startBackendIfNeeded()
    createMainWindow()

    app.on("activate", () => {
      if (BrowserWindow.getAllWindows().length === 0) createMainWindow()
    })
  })
}

app.on("window-all-closed", () => {
  if (process.platform !== "darwin") app.quit()
})

app.on("before-quit", () => {
  backendProcess?.kill()
})
