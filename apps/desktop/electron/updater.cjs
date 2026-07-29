let mainWindow = null;
let autoUpdater = null;

function getAutoUpdater() {
  if (!autoUpdater) {
    autoUpdater = require("electron-updater").autoUpdater;
  }
  return autoUpdater;
}

function setupAutoUpdater(window) {
  mainWindow = window;

  const updater = getAutoUpdater();
  updater.autoDownload = false;
  updater.autoInstallOnAppQuit = true;

  updater.on("checking-for-update", () => {
    sendStatus("checking-for-update");
  });

  updater.on("update-available", (info) => {
    sendStatus("update-available", {
      version: info.version,
      releaseDate: info.releaseDate,
    });
    updater.downloadUpdate();
  });

  updater.on("update-not-available", (info) => {
    sendStatus("update-not-available", {
      version: info.version,
    });
  });

  updater.on("download-progress", (progress) => {
    sendStatus("download-progress", {
      percent: progress.percent,
      bytesPerSecond: progress.bytesPerSecond,
      transferred: progress.transferred,
      total: progress.total,
    });
  });

  updater.on("update-downloaded", (info) => {
    sendStatus("update-downloaded", {
      version: info.version,
      releaseDate: info.releaseDate,
    });
  });

  updater.on("error", (error) => {
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
  getAutoUpdater().checkForUpdates().catch(() => {
    // Silently ignore update check failures
  });
}

function quitAndInstall() {
  getAutoUpdater().quitAndInstall();
}

module.exports = {
  setupAutoUpdater,
  checkForUpdates,
  quitAndInstall,
};
