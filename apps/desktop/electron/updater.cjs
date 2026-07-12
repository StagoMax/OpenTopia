const { autoUpdater } = require("electron-updater");
const { BrowserWindow } = require("electron");

let mainWindow = null;

function setupAutoUpdater(window) {
  mainWindow = window;

  autoUpdater.autoDownload = false;
  autoUpdater.autoInstallOnAppQuit = true;

  autoUpdater.on("checking-for-update", () => {
    sendStatus("checking-for-update");
  });

  autoUpdater.on("update-available", (info) => {
    sendStatus("update-available", {
      version: info.version,
      releaseDate: info.releaseDate,
    });
    autoUpdater.downloadUpdate();
  });

  autoUpdater.on("update-not-available", (info) => {
    sendStatus("update-not-available", {
      version: info.version,
    });
  });

  autoUpdater.on("download-progress", (progress) => {
    sendStatus("download-progress", {
      percent: progress.percent,
      bytesPerSecond: progress.bytesPerSecond,
      transferred: progress.transferred,
      total: progress.total,
    });
  });

  autoUpdater.on("update-downloaded", (info) => {
    sendStatus("update-downloaded", {
      version: info.version,
      releaseDate: info.releaseDate,
    });
  });

  autoUpdater.on("error", (error) => {
    sendStatus("error", {
      message: error == null ? "unknown" : (error.message || error).toString(),
    });
  });
}

function sendStatus(status, data) {
  if (!mainWindow || mainWindow.webContents.isDestroyed()) return;
  mainWindow.webContents.send("updater:status", { status, data });
}

function checkForUpdates() {
  autoUpdater.checkForUpdates().catch(() => {
    // Silently ignore update check failures
  });
}

function quitAndInstall() {
  autoUpdater.quitAndInstall();
}

module.exports = {
  setupAutoUpdater,
  checkForUpdates,
  quitAndInstall,
};
