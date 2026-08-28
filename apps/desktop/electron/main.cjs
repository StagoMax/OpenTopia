const {
  app,
  BrowserWindow,
  WebContentsView,
  clipboard,
  dialog,
  ipcMain,
  Menu,
  Notification,
  nativeImage,
  safeStorage,
  shell,
} = require("electron");
const path = require("node:path");
const { URL, fileURLToPath } = require("node:url");
const { spawn, spawnSync } = require("node:child_process");
const crypto = require("node:crypto");
const fs = require("node:fs");
const net = require("node:net");
const spreadsheetFormats = require("./spreadsheet-formats.json");
const updater = require("./updater.cjs");
const { createDesktopBrowserHost } = require("./browser-host.cjs");
const {
  createBackendEventStreamManager,
} = require("./backend-event-stream.cjs");
const { createChromeBridge } = require("./chrome-bridge.cjs");
const { createAppLogger } = require("./logging.cjs");
const {
  DEFAULT_SAG_URL,
  createSagServiceManager,
} = require("./sag-service.cjs");
const {
  DEFAULT_GRAPH_RAG_URL,
  createGraphRagServiceManager,
} = require("./graph-rag-service.cjs");
const {
  inspectSandboxProtocol,
  loadRuntimeBundle,
  runtimeManifestName,
} = require("./runtime-bundle.cjs");
const {
  applyAgentToolsEnvironment,
  resolveDevelopmentAgentToolsRuntime,
} = require("./agent-tools-runtime.cjs");
const {
  autostartDeploymentLibraryServices,
} = require("./deployment-library-autostart.cjs");
const {
  createDesktopToolMenuTemplate,
  normalizeDesktopToolMenuRequest,
} = require("./desktop-tool-menu.cjs");

const isDev = !app.isPackaged;
if (isDev) {
  app.setName("OpenTopia Dev");
  const devUserDataPath =
    process.env.OPENTOPIA_DEV_USER_DATA ||
    path.join(app.getPath("appData"), "OpenTopia Dev");
  app.setPath("userData", path.resolve(devUserDataPath));
}
const hasExplicitBackendUrl = Boolean(process.env.OPENTOPIA_SERVER_URL);
let defaultBackendUrl =
  process.env.OPENTOPIA_SERVER_URL || "http://127.0.0.1:8787";
// Keep this in lockstep with the health response in opentopia-server. A health
// response is also the desktop client/server compatibility boundary: a server
// from an earlier build must not be reused merely because it is listening.
const desktopBackendApiVersion = 2;
const backendApiToken = crypto.randomBytes(32).toString("base64url");
const openTopiaProtocol = "opentopia";
const {
  backendEndpointInfo,
  ensureLoggingInitialized,
  flushLogsSync,
  getLogPaths,
  isSecretName,
  logConsole,
  redactSecrets,
  serializeError,
  writeLog,
} = createAppLogger({
  app,
  apiToken: backendApiToken,
  getBackendUrl: () => defaultBackendUrl,
  isDev,
});
const backendEventStreamManager = createBackendEventStreamManager({
  getBackendUrl: () => defaultBackendUrl,
  getApiToken: () => backendApiToken,
  logger: (level, event, metadata) => writeLog(level, event, metadata),
});

/*
 * The Windows caption buttons are drawn by the OS above the renderer. Keep the
 * overlay transparent so the time-aware topbar remains visible underneath,
 * while the symbols still follow the resolved theme for contrast.
 */
const titleBarOverlayColors = {
  light: { color: "rgba(1, 0, 0, 0)", symbolColor: "#5c6570" },
  dark: { color: "rgba(1, 0, 0, 0)", symbolColor: "#c2c2c2" },
};
const windowBackgroundColors = { light: "#ffffff", dark: "#181818" };

function titleBarOverlayFor(theme) {
  const palette = titleBarOverlayColors[theme] ?? titleBarOverlayColors.light;
  return { ...palette, height: 32 };
}

let mainWindow = null;
const appWindows = new Set();
let backendProcess = null;
let backendStartupStatus = {
  phase: "checking",
  detail: null,
  startedAt: new Date().toISOString(),
  updatedAt: new Date().toISOString(),
};
let protocolClientRegistered = false;
let nextOpenRequestId = 1;
let desktopBrowserHost = null;
let desktopBrowserBroker = null;
let chromeBridge = null;
let chromeBridgeBackend = null;
let sagServiceManager = null;
let graphRagServiceManager = null;
let packagedRuntimeBundle = null;
let packagedRuntimeBundleError = null;
let appQuitPrepared = false;
let appQuitPreparation = null;

function backendStartupStatusSnapshot() {
  return { ...backendStartupStatus };
}

function publishBackendStartupStatus() {
  for (const appWindow of appWindows) {
    if (appWindow.isDestroyed() || appWindow.webContents.isDestroyed()) {
      continue;
    }
    appWindow.webContents.send(
      "backend:startup-status",
      backendStartupStatusSnapshot(),
    );
  }
}

function updateBackendStartupStatus(
  phase,
  { detail = null, resetElapsed = false } = {},
) {
  const now = new Date().toISOString();
  backendStartupStatus = {
    phase,
    detail,
    startedAt: resetElapsed ? now : backendStartupStatus.startedAt,
    updatedAt: now,
  };
  publishBackendStartupStatus();
}

const secretsFilePath = "secrets.json";
const providerSecretStorageKey = "provider-api-key";
const providerSecretStoragePrefix = `${providerSecretStorageKey}:`;
const keyringProviderApiKeySourceId = `keyring:${providerSecretStorageKey}`;
const keyringProviderApiKeyEnvName = "OPENTOPIA_API_KEY";

const maxRecentWorkspaces = 12;
const maxContextSourceFiles = 20;
const maxContextSourceBytes = 25 * 1024 * 1024;
const spreadsheetContextExtensions = Object.freeze([
  ...spreadsheetFormats.extensions,
]);
const delimitedSpreadsheetContextExtensions = new Set(
  spreadsheetFormats.delimitedExtensions,
);
const workbookContextExtensions = new Set(
  spreadsheetContextExtensions.filter(
    (extension) => !delimitedSpreadsheetContextExtensions.has(extension),
  ),
);
const supportedContextFileExtensions = Object.freeze([
  "txt",
  "md",
  "json",
  "jsonc",
  "jsonl",
  ...spreadsheetContextExtensions,
  "yaml",
  "yml",
  "toml",
  "xml",
  "html",
  "css",
  "scss",
  "less",
  "js",
  "jsx",
  "ts",
  "tsx",
  "rs",
  "py",
  "go",
  "java",
  "kt",
  "swift",
  "rb",
  "php",
  "sql",
  "graphql",
  "gql",
  "proto",
  "diff",
  "patch",
  "c",
  "h",
  "cpp",
  "hpp",
  "png",
  "jpg",
  "jpeg",
  "gif",
  "webp",
  "bmp",
  "pdf",
  "docx",
  "pptx",
]);
const recentWorkspacesFile = "recent-workspaces.json";
const openRequestHistoryLimit = 50;
const openRequestHistory = [];
const maxSystemNotificationTitleLength = 120;
const maxSystemNotificationBodyLength = 1_000;
const maxRetainedSystemNotifications = 20;
const activeSystemNotifications = new Set();
const providerSecretEnvNames = [
  "OPENTOPIA_API_KEY",
  "OPENAI_API_KEY",
  "CREDIT_REVIEW_LLM_API_KEY",
  "AUDIT_COPILOT_LLM_API_KEY",
];

function prependPath(env, entry) {
  if (!entry || !fs.existsSync(entry)) return;

  const pathKey =
    Object.keys(env).find((key) => key.toLowerCase() === "path") || "PATH";
  const current = env[pathKey] || "";
  const entries = current.split(path.delimiter).filter(Boolean);
  const normalizedEntry = entry.toLowerCase();
  const alreadyPresent = entries.some(
    (candidate) => candidate.toLowerCase() === normalizedEntry,
  );
  if (!alreadyPresent) {
    env[pathKey] = [entry, ...entries].join(path.delimiter);
  }
}

function resolveMingwBin() {
  if (
    process.env.OPENTOPIA_MINGW_BIN &&
    fs.existsSync(process.env.OPENTOPIA_MINGW_BIN)
  ) {
    return process.env.OPENTOPIA_MINGW_BIN;
  }

  const localAppData = process.env.LOCALAPPDATA;
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
  ].filter(Boolean);

  return (
    candidates.find((candidate) =>
      fs.existsSync(path.join(candidate, "gcc.exe")),
    ) || null
  );
}

function stripEnvValue(value) {
  const trimmed = value.trim();
  if (trimmed.length >= 2) {
    const first = trimmed[0];
    const last = trimmed[trimmed.length - 1];
    if ((first === '"' && last === '"') || (first === "'" && last === "'")) {
      return trimmed.slice(1, -1);
    }
  }
  return trimmed;
}

function importEnvFile(env, filePath) {
  if (!filePath || !fs.existsSync(filePath)) return false;

  const content = fs.readFileSync(filePath, "utf8");
  for (const rawLine of content.split(/\r?\n/)) {
    let line = rawLine.trim();
    if (!line || line.startsWith("#")) continue;
    if (line.startsWith("export ")) line = line.slice(7).trim();

    const separator = line.indexOf("=");
    if (separator <= 0) continue;

    const key = line.slice(0, separator).trim();
    const value = stripEnvValue(line.slice(separator + 1));
    if (key && !env[key]) env[key] = value;
  }

  env.OPENTOPIA_ENV_FILE ||= filePath;
  return true;
}

function resolveOpenTopiaEnvFile(repoRoot) {
  if (
    process.env.OPENTOPIA_ENV_FILE &&
    fs.existsSync(process.env.OPENTOPIA_ENV_FILE)
  ) {
    return process.env.OPENTOPIA_ENV_FILE;
  }

  const localEnv = path.join(repoRoot, ".env");
  if (fs.existsSync(localEnv)) return localEnv;

  const workspaceRoot = path.dirname(repoRoot);
  const creditReviewProjectName = String.fromCodePoint(
    0x4fe1,
    0x8d37,
    0x5ba1,
    0x6838,
    0x52a9,
    0x624b,
  );
  const creditReviewEnv = path.join(
    workspaceRoot,
    creditReviewProjectName,
    ".env",
  );
  if (fs.existsSync(creditReviewEnv)) return creditReviewEnv;

  const markers = ["CREDIT_REVIEW_LLM_API_KEY", "AUDIT_COPILOT_LLM_API_KEY"];
  try {
    for (const entry of fs.readdirSync(workspaceRoot, {
      withFileTypes: true,
    })) {
      if (!entry.isDirectory()) continue;

      const candidate = path.join(workspaceRoot, entry.name, ".env");
      if (!fs.existsSync(candidate)) continue;

      const content = fs.readFileSync(candidate, "utf8");
      if (markers.some((marker) => content.includes(marker))) return candidate;
    }
  } catch {
    return null;
  }

  return null;
}

function importProviderCredentialFallback(env, repoRoot, selectedEnvFile) {
  // A generic OPENAI_API_KEY may belong to a different endpoint. It must not
  // prevent a workspace-specific credential from being loaded for a custom
  // OpenAI-compatible base URL.
  const explicitProviderSecretNames = [
    "OPENTOPIA_API_KEY",
    "CREDIT_REVIEW_LLM_API_KEY",
    "AUDIT_COPILOT_LLM_API_KEY",
  ];
  if (explicitProviderSecretNames.some((name) => Boolean(env[name]))) return;

  const workspaceRoot = path.dirname(repoRoot);
  const creditReviewProjectName = String.fromCodePoint(
    0x4fe1,
    0x8d37,
    0x5ba1,
    0x6838,
    0x52a9,
    0x624b,
  );
  const preferred = path.join(workspaceRoot, creditReviewProjectName, ".env");
  const candidates = [preferred];
  try {
    for (const entry of fs.readdirSync(workspaceRoot, {
      withFileTypes: true,
    })) {
      if (!entry.isDirectory()) continue;
      const candidate = path.join(workspaceRoot, entry.name, ".env");
      if (candidate !== preferred) candidates.push(candidate);
    }
  } catch {
    // The preferred sibling path is still checked below.
  }

  const markers = ["CREDIT_REVIEW_LLM_API_KEY", "AUDIT_COPILOT_LLM_API_KEY"];
  for (const candidate of candidates) {
    if (candidate === selectedEnvFile || !fs.existsSync(candidate)) continue;
    const content = fs.readFileSync(candidate, "utf8");
    if (!markers.some((marker) => content.includes(marker))) continue;
    importEnvFile(env, candidate);
    if (explicitProviderSecretNames.some((name) => Boolean(env[name]))) return;
  }
}

function applyProviderAliases(env) {
  const setFromAliases = (target, aliases) => {
    if (env[target]) return;
    for (const alias of aliases) {
      if (env[alias]) {
        env[target] = env[alias];
        return;
      }
    }
  };

  setFromAliases("OPENTOPIA_API_KEY", [
    "AUDIT_COPILOT_LLM_API_KEY",
    "CREDIT_REVIEW_LLM_API_KEY",
    "OPENAI_API_KEY",
  ]);
  setFromAliases("OPENTOPIA_OPENAI_BASE_URL", [
    "AUDIT_COPILOT_LLM_BASE_URL",
    "CREDIT_REVIEW_LLM_BASE_URL",
    "OPENAI_BASE_URL",
  ]);
  setFromAliases("OPENTOPIA_MODEL", [
    "AUDIT_COPILOT_LLM_MODEL",
    "CREDIT_REVIEW_LLM_MODEL",
    "CREDIT_REVIEW_LLM_CHEAP_MODEL",
    "CREDIT_REVIEW_LLM_STRONG_MODEL",
  ]);
}

function secretsPath() {
  return path.join(app.getPath("userData"), secretsFilePath);
}

function emptySecretStore() {
  return {
    version: 1,
    secrets: {},
  };
}

function normalizeProviderSecretKey(key) {
  const rawKey = String(key || "").trim();
  const allowedKeys = new Set([
    providerSecretStorageKey,
    keyringProviderApiKeySourceId,
    keyringProviderApiKeyEnvName,
    `env:${keyringProviderApiKeyEnvName}`,
    ...providerSecretEnvNames,
    ...providerSecretEnvNames.map((envName) => `env:${envName}`),
  ]);

  if (!allowedKeys.has(rawKey)) {
    throw new Error("Only the provider API key can be stored in keyring");
  }

  return providerSecretStorageKey;
}

function normalizeStoredProviderSecretKey(key) {
  try {
    return normalizeProviderSecretKey(key);
  } catch {
    return null;
  }
}

function readSecretStore() {
  try {
    const parsed = JSON.parse(fs.readFileSync(secretsPath(), "utf8"));
    if (!parsed || typeof parsed !== "object") return emptySecretStore();

    if (
      parsed.version === 1 &&
      parsed.secrets &&
      typeof parsed.secrets === "object"
    ) {
      return {
        version: 1,
        secrets: parsed.secrets,
      };
    }

    const migrated = emptySecretStore();
    for (const [key, encryptedHex] of Object.entries(parsed)) {
      const normalizedKey = normalizeStoredProviderSecretKey(key);
      if (!normalizedKey || typeof encryptedHex !== "string") continue;
      migrated.secrets[normalizedKey] = {
        kind: "safeStorage",
        envTarget: keyringProviderApiKeyEnvName,
        encryptedHex,
        updatedAt: null,
      };
    }
    return migrated;
  } catch (error) {
    if (error?.code !== "ENOENT") {
      logConsole("warn", "secrets.read.failed", { error });
    }
    return emptySecretStore();
  }
}

function writeSecretStore(store) {
  const targetPath = secretsPath();
  fs.mkdirSync(path.dirname(targetPath), { recursive: true });
  fs.writeFileSync(targetPath, `${JSON.stringify(store, null, 2)}\n`, {
    encoding: "utf8",
    mode: 0o600,
  });
  try {
    fs.chmodSync(targetPath, 0o600);
  } catch {
    // Best effort only; Windows ACLs are controlled by the user profile.
  }
}

function providerApiKeySecretEntry() {
  return readSecretStore().secrets[providerSecretStorageKey] || null;
}

function normalizeProviderId(providerId) {
  const value = String(providerId || "").trim();
  if (!/^[A-Za-z0-9][A-Za-z0-9._-]{0,79}$/.test(value)) {
    throw new Error(
      "Provider ID must start with a letter or number and contain only letters, numbers, dots, underscores, or hyphens",
    );
  }
  return value;
}

function providerSecretStorageKeyFor(providerId) {
  return `${providerSecretStoragePrefix}${normalizeProviderId(providerId)}`;
}

function providerSecretSourceId(providerId) {
  return `keyring:${providerSecretStorageKeyFor(providerId)}`;
}

function providerSecretEnvTarget(providerId) {
  const digest = crypto
    .createHash("sha256")
    .update(normalizeProviderId(providerId))
    .digest("hex")
    .slice(0, 16)
    .toUpperCase();
  return `OPENTOPIA_PROVIDER_${digest}_API_KEY`;
}

function isProviderApiKeyConfigured() {
  return Boolean(providerApiKeySecretEntry()?.encryptedHex);
}

function selectedSafeStorageBackend() {
  try {
    return typeof safeStorage.getSelectedStorageBackend === "function"
      ? safeStorage.getSelectedStorageBackend()
      : null;
  } catch {
    return null;
  }
}

function keyringMetadata() {
  const encryptionAvailable = safeStorage.isEncryptionAvailable();
  const providerApiKeyConfigured = isProviderApiKeyConfigured();
  const status = !encryptionAvailable
    ? providerApiKeyConfigured
      ? "configured_unavailable"
      : "unavailable"
    : providerApiKeyConfigured
      ? "available"
      : "not_configured";

  return {
    available: encryptionAvailable,
    encryptionAvailable,
    storageBackend: selectedSafeStorageBackend(),
    storagePath: secretsPath(),
    providerApiKeyConfigured,
    providerApiKeySourceId: keyringProviderApiKeySourceId,
    envTarget: keyringProviderApiKeyEnvName,
    status,
  };
}

function readProviderApiKeySecret() {
  if (!safeStorage.isEncryptionAvailable()) return null;

  const entry = providerApiKeySecretEntry();
  if (!entry?.encryptedHex) return null;

  try {
    return safeStorage.decryptString(Buffer.from(entry.encryptedHex, "hex"));
  } catch (error) {
    logConsole("warn", "secrets.provider.decrypt.failed", { error });
    return null;
  }
}

function decryptSecretEntry(entry) {
  if (!safeStorage.isEncryptionAvailable() || !entry?.encryptedHex) return null;
  try {
    return safeStorage.decryptString(Buffer.from(entry.encryptedHex, "hex"));
  } catch (error) {
    logConsole("warn", "secrets.provider.decrypt.failed", { error });
    return null;
  }
}

function injectKeyringProviderApiKey(env) {
  const value = readProviderApiKeySecret();
  // Legacy single-key storage is a fallback only. New user-entered credentials
  // use provider-specific env targets below and always win for that profile.
  if (value && !env[keyringProviderApiKeyEnvName]) {
    env[keyringProviderApiKeyEnvName] = value;
  }

  for (const [storageKey, entry] of Object.entries(readSecretStore().secrets)) {
    if (!storageKey.startsWith(providerSecretStoragePrefix)) continue;
    const providerId = storageKey.slice(providerSecretStoragePrefix.length);
    let envTarget;
    try {
      envTarget = entry.envTarget || providerSecretEnvTarget(providerId);
    } catch {
      continue;
    }
    const providerValue = decryptSecretEntry(entry);
    if (providerValue) env[envTarget] = providerValue;
  }
}

function setProviderApiKeySecret(value) {
  if (!safeStorage.isEncryptionAvailable()) {
    throw new Error("Encryption not available on this system");
  }

  const secretValue = String(value || "").trim();
  if (!secretValue) {
    throw new Error("Provider API key cannot be empty");
  }

  const store = readSecretStore();
  const encrypted = safeStorage.encryptString(secretValue);
  store.secrets[providerSecretStorageKey] = {
    kind: "safeStorage",
    envTarget: keyringProviderApiKeyEnvName,
    encryptedHex: encrypted.toString("hex"),
    updatedAt: new Date().toISOString(),
  };
  writeSecretStore(store);
  return keyringMetadata();
}

function deleteProviderApiKeySecret() {
  const store = readSecretStore();
  delete store.secrets[providerSecretStorageKey];
  writeSecretStore(store);
  return keyringMetadata();
}

function providerKeyringMetadata(providerId) {
  const normalizedId = normalizeProviderId(providerId);
  const storageKey = providerSecretStorageKeyFor(normalizedId);
  const configured = Boolean(
    readSecretStore().secrets[storageKey]?.encryptedHex,
  );
  const encryptionAvailable = safeStorage.isEncryptionAvailable();
  return {
    providerId: normalizedId,
    available: encryptionAvailable,
    encryptionAvailable,
    storageBackend: selectedSafeStorageBackend(),
    storagePath: secretsPath(),
    providerApiKeyConfigured: configured,
    providerApiKeySourceId: providerSecretSourceId(normalizedId),
    envTarget: providerSecretEnvTarget(normalizedId),
    status: !encryptionAvailable
      ? configured
        ? "configured_unavailable"
        : "unavailable"
      : configured
        ? "available"
        : "not_configured",
  };
}

function setProviderKeyringSecret(providerId, value) {
  if (!safeStorage.isEncryptionAvailable()) {
    throw new Error("Encryption not available on this system");
  }
  const metadata = providerKeyringMetadata(providerId);
  const secretValue = String(value || "").trim();
  if (!secretValue) throw new Error("Provider API key cannot be empty");
  const store = readSecretStore();
  store.secrets[providerSecretStorageKeyFor(metadata.providerId)] = {
    kind: "safeStorage",
    envTarget: metadata.envTarget,
    encryptedHex: safeStorage.encryptString(secretValue).toString("hex"),
    updatedAt: new Date().toISOString(),
  };
  writeSecretStore(store);
  return providerKeyringMetadata(metadata.providerId);
}

function deleteProviderKeyringSecret(providerId) {
  const metadata = providerKeyringMetadata(providerId);
  const store = readSecretStore();
  delete store.secrets[providerSecretStorageKeyFor(metadata.providerId)];
  writeSecretStore(store);
  return providerKeyringMetadata(metadata.providerId);
}

function createBackendEnv(repoRoot, options = {}) {
  const defaultDatabasePath = isDev
    ? path.join(repoRoot, ".opentopia", "opentopia.db")
    : path.join(app.getPath("userData"), "opentopia.db");
  const defaultArtifactsPath = isDev
    ? path.join(repoRoot, ".opentopia", "artifacts")
    : path.join(app.getPath("userData"), "artifacts");
  const env = {
    ...process.env,
    OPENTOPIA_DB: process.env.OPENTOPIA_DB || defaultDatabasePath,
    OPENTOPIA_ARTIFACTS_DIR:
      process.env.OPENTOPIA_ARTIFACTS_DIR || defaultArtifactsPath,
    OPENTOPIA_PERMISSION: process.env.OPENTOPIA_PERMISSION || "auto",
    OPENTOPIA_API_TOKEN: backendApiToken,
  };

  if (isDev) {
    env.CARGO_TARGET_DIR ||=
      process.env.OPENTOPIA_DEV_CARGO_TARGET_DIR ||
      path.join(repoRoot, "target", "desktop-dev");
  }

  let agentToolsRuntime = null;
  if (!isDev) {
    const bundle = resolvePackagedRuntimeBundle();
    if (bundle?.officeRuntimeRoot) {
      env.OPENTOPIA_OFFICE_RUNTIME_ROOT = bundle.officeRuntimeRoot;
    }
    agentToolsRuntime = bundle?.agentToolsRuntime || null;
  }

  if (desktopBrowserBroker) {
    env.OPENTOPIA_DESKTOP_BROWSER_BROKER_URL = desktopBrowserBroker.url;
    env.OPENTOPIA_DESKTOP_BROWSER_BROKER_TOKEN = desktopBrowserBroker.token;
  }

  if (chromeBridgeBackend) {
    env.OPENTOPIA_CHROME_BRIDGE_URL = chromeBridgeBackend.url;
    env.OPENTOPIA_CHROME_BRIDGE_TOKEN = chromeBridgeBackend.token;
  }

  if (isDev) {
    env.OPENTOPIA_DEV_ORIGIN =
      process.env.VITE_DEV_SERVER_URL || "http://127.0.0.1:5173";
  }

  const selectedEnvFile = resolveOpenTopiaEnvFile(repoRoot);
  importEnvFile(env, selectedEnvFile);
  importProviderCredentialFallback(env, repoRoot, selectedEnvFile);
  applyProviderAliases(env);
  if (isDev) {
    agentToolsRuntime = resolveDevelopmentAgentToolsRuntime(repoRoot, env);
  }
  if (options.includeKeyring !== false) {
    injectKeyringProviderApiKey(env);
    if (process.platform === "win32") {
      const sandbox = resolveOpenTopiaWindowsSandboxBinary(repoRoot);
      if (sandbox.exists) {
        env.OPENTOPIA_WINDOWS_SANDBOX_BIN = sandbox.path;
        writeLog("info", "sandbox.helper.selected", {
          path: sandbox.path,
          mode: isDev ? "development" : "packaged",
        });
      } else if (sandbox.reason) {
        env.OPENTOPIA_SANDBOX_BACKEND_ERROR = sandbox.reason;
        writeLog("error", "sandbox.helper.missing", {
          path: sandbox.path,
          reason: sandbox.reason,
        });
      }
    }
    env.OPENTOPIA_SANDBOX_MODE ||= "workspace-write";
    env.OPENTOPIA_SANDBOX_ENFORCEMENT ||=
      process.env.OPENTOPIA_SANDBOX_ENFORCEMENT || "enforce";
    env.OPENTOPIA_SANDBOX_NETWORK ||= "deny";
  }

  if (process.platform === "win32") {
    env.RUSTUP_TOOLCHAIN =
      process.env.OPENTOPIA_RUST_TOOLCHAIN ||
      process.env.RUSTUP_TOOLCHAIN ||
      "stable-x86_64-pc-windows-msvc";
    if (process.env.USERPROFILE)
      prependPath(env, path.join(process.env.USERPROFILE, ".cargo", "bin"));
    prependPath(env, resolveMingwBin());
  }

  applyAgentToolsEnvironment(env, agentToolsRuntime);

  return env;
}

function workspaceName(workspaceRoot) {
  const parsed = path.parse(workspaceRoot);
  const trimmed = workspaceRoot.replace(/[\\\/]+$/, "");
  return path.basename(trimmed) || parsed.root || workspaceRoot;
}

function workspaceKey(workspaceRoot) {
  return process.platform === "win32"
    ? workspaceRoot.toLowerCase()
    : workspaceRoot;
}

function normalizeExistingPath(rawPath) {
  if (typeof rawPath !== "string" || rawPath.trim() === "") {
    throw new Error("Path must be a non-empty string.");
  }

  const resolvedPath = path.resolve(rawPath);
  if (!fs.existsSync(resolvedPath)) {
    throw new Error(`Path does not exist: ${resolvedPath}`);
  }

  return (
    fs.realpathSync.native?.(resolvedPath) || fs.realpathSync(resolvedPath)
  );
}

function normalizeExistingFile(rawPath) {
  const filePath = normalizeExistingPath(rawPath);
  if (!fs.statSync(filePath).isFile()) {
    throw new Error(`Path is not a file: ${filePath}`);
  }
  return filePath;
}

function userVisibleWindowsPath(filePath) {
  const verbatimUncPrefix = "\\\\?\\UNC\\";
  if (
    filePath.slice(0, verbatimUncPrefix.length).toUpperCase() ===
    verbatimUncPrefix
  ) {
    return `\\\\${filePath.slice(verbatimUncPrefix.length)}`;
  }
  return filePath.startsWith("\\\\?\\") ? filePath.slice(4) : filePath;
}

function visualStudioCodeFileUrl(filePath, line) {
  const normalized = userVisibleWindowsPath(filePath).replaceAll("\\", "/");
  const encoded = encodeURI(normalized)
    .replaceAll("#", "%23")
    .replaceAll("?", "%3F");
  const location = Number.isInteger(line) && line > 0 ? `:${line}` : "";
  return `vscode://file/${encoded}${location}`;
}

async function showOpenWithDialog(filePath) {
  if (process.platform !== "win32") {
    const error = await shell.openPath(filePath);
    if (error) throw new Error(error);
    return;
  }

  await new Promise((resolve, reject) => {
    const child = spawn(
      "rundll32.exe",
      ["shell32.dll,OpenAs_RunDLL", filePath],
      { detached: true, stdio: "ignore", windowsHide: true },
    );
    child.once("error", reject);
    child.once("spawn", () => {
      child.unref();
      resolve();
    });
  });
}

function normalizeComparablePath(rawPath) {
  if (typeof rawPath !== "string" || rawPath.trim() === "") {
    throw new Error("Path must be a non-empty string.");
  }

  const resolvedPath = path.resolve(rawPath);
  if (!fs.existsSync(resolvedPath)) return resolvedPath;
  return (
    fs.realpathSync.native?.(resolvedPath) || fs.realpathSync(resolvedPath)
  );
}

function normalizeWorkspaceRoot(rawPath) {
  const workspaceRoot = normalizeExistingPath(rawPath);
  const stat = fs.statSync(workspaceRoot);
  if (!stat.isDirectory()) {
    throw new Error(`Workspace must be a directory: ${workspaceRoot}`);
  }
  return workspaceRoot;
}

function contextSourceKind(filePath) {
  const extension = path.extname(filePath).slice(1).toLowerCase();
  if (["png", "jpg", "jpeg", "gif", "webp", "bmp"].includes(extension)) {
    return "image";
  }
  if (
    ["pdf", "docx", "pptx"].includes(extension) ||
    workbookContextExtensions.has(extension)
  ) {
    return "document";
  }
  return "text";
}

function contextSourceMetadata(rawPath) {
  const filePath = normalizeExistingPath(rawPath);
  const stat = fs.statSync(filePath);
  if (!stat.isFile())
    throw new Error(`Context source must be a file: ${filePath}`);
  if (stat.size > maxContextSourceBytes) {
    throw new Error(
      `Context source exceeds ${maxContextSourceBytes} bytes: ${filePath}`,
    );
  }
  return {
    path: filePath,
    name: path.basename(filePath),
    extension: path.extname(filePath).toLowerCase(),
    kind: contextSourceKind(filePath),
    bytes: stat.size,
  };
}

function resolvePathArgument(rawPath, cwd) {
  let candidate = String(rawPath || "").trim();
  if (!candidate) throw new Error("Path argument is empty.");

  if (candidate.startsWith("file://")) {
    candidate = fileURLToPath(candidate);
  }

  const resolvedPath = path.isAbsolute(candidate)
    ? candidate
    : path.resolve(cwd || process.cwd(), candidate);
  const exists = fs.existsSync(resolvedPath);
  const realPath = exists
    ? fs.realpathSync.native?.(resolvedPath) || fs.realpathSync(resolvedPath)
    : resolvedPath;
  const stat = exists ? fs.statSync(realPath) : null;
  return {
    path: realPath,
    exists,
    isDirectory: Boolean(stat?.isDirectory()),
    isFile: Boolean(stat?.isFile()),
  };
}

function toOpenRequestId() {
  const suffix = String(nextOpenRequestId).padStart(4, "0");
  nextOpenRequestId += 1;
  return `${Date.now()}-${suffix}`;
}

function createOpenRequest(source, kind, payload) {
  return {
    id: toOpenRequestId(),
    source,
    kind,
    receivedAt: new Date().toISOString(),
    ...payload,
  };
}

function safeDeepLinkParams(searchParams) {
  const params = {};
  for (const [key, value] of searchParams.entries()) {
    params[key] = isSecretName(key) ? "[redacted]" : value;
  }
  return params;
}

function parseDeepLinkOpenRequest(rawUrl, source, cwd) {
  const parsed = new URL(rawUrl);
  if (parsed.protocol !== `${openTopiaProtocol}:`) return null;

  const action =
    parsed.hostname || parsed.pathname.replace(/^\/+/, "") || "open";
  const request = createOpenRequest(source, "deeplink", {
    protocol: openTopiaProtocol,
    action,
    url: redactSecrets(parsed.toString()),
    params: safeDeepLinkParams(parsed.searchParams),
  });

  const threadId = parsed.searchParams.get("thread");
  if (threadId && threadId.length <= 200) request.threadId = threadId;

  const targetPath =
    parsed.searchParams.get("workspace") || parsed.searchParams.get("path");
  if (targetPath) {
    try {
      const target = resolvePathArgument(targetPath, cwd);
      request.target = {
        path: target.path,
        exists: target.exists,
        kind: target.isDirectory
          ? "workspace"
          : target.isFile
            ? "file"
            : "path",
      };
      if (target.isDirectory) request.workspaceRoot = target.path;
      else request.path = target.path;
    } catch (error) {
      request.error =
        serializeError(error)?.message || "Invalid path argument.";
    }
  }

  return request;
}

function parseFileOpenRequest(rawPath, source, cwd, preferredKind) {
  const target = resolvePathArgument(rawPath, cwd);
  const kind =
    preferredKind ||
    (target.isDirectory ? "workspace" : target.isFile ? "file" : "path");
  const payload = {
    path: target.path,
    exists: target.exists,
  };
  if (kind === "workspace" || (kind === "folder" && target.isDirectory)) {
    payload.workspaceRoot = target.path;
  }
  return createOpenRequest(source, kind, payload);
}

function openArgPreferredKind(flag) {
  switch (flag) {
    case "--workspace":
      return "workspace";
    case "--folder":
    case "--directory":
      return "folder";
    case "--file":
      return "file";
    case "--path":
    case "--open":
      return null;
    default:
      return null;
  }
}

function isLikelyPathArgument(value, cwd) {
  if (!value || value.startsWith("-")) return false;
  if (value.startsWith("file://")) return true;
  if (path.isAbsolute(value)) return fs.existsSync(value);
  return fs.existsSync(path.resolve(cwd || process.cwd(), value));
}

function extractOpenArgs(argv, cwd) {
  const args = Array.isArray(argv) ? argv : [];
  const startIndex = isDev ? 2 : 1;
  const values = [];
  for (let index = startIndex; index < args.length; index += 1) {
    const arg = args[index];
    if (!arg || arg === "--") continue;

    const equalsIndex = arg.indexOf("=");
    if (equalsIndex > 0) {
      const flag = arg.slice(0, equalsIndex);
      const preferredKind = openArgPreferredKind(flag);
      if (preferredKind !== null || flag === "--open" || flag === "--path") {
        values.push({
          value: arg.slice(equalsIndex + 1),
          preferredKind,
        });
        continue;
      }
    }

    const preferredKind = openArgPreferredKind(arg);
    if (
      preferredKind !== null ||
      arg === "--open" ||
      arg === "--path" ||
      arg === "--file"
    ) {
      const value = args[index + 1];
      if (value) {
        values.push({ value, preferredKind });
        index += 1;
      }
      continue;
    }

    if (
      arg.startsWith(`${openTopiaProtocol}://`) ||
      arg.startsWith("file://") ||
      isLikelyPathArgument(arg, cwd)
    ) {
      values.push({ value: arg, preferredKind: null });
    }
  }
  return values;
}

function queueOpenRequestFromValue(
  source,
  rawValue,
  cwd,
  preferredKind = null,
) {
  if (typeof rawValue !== "string" || rawValue.trim() === "") return null;

  try {
    const value = rawValue.trim();
    const request = value.startsWith(`${openTopiaProtocol}://`)
      ? parseDeepLinkOpenRequest(value, source, cwd)
      : parseFileOpenRequest(value, source, cwd, preferredKind);
    if (!request) return null;
    enqueueOpenRequest(request);
    return request;
  } catch (error) {
    const request = createOpenRequest(source, "path", {
      path: String(rawValue),
      exists: false,
      error: serializeError(error)?.message || "Failed to parse open request.",
    });
    enqueueOpenRequest(request);
    return request;
  }
}

function queueOpenRequestsFromArgv(source, argv, cwd) {
  const requests = [];
  for (const candidate of extractOpenArgs(argv, cwd)) {
    const request = queueOpenRequestFromValue(
      source,
      candidate.value,
      cwd,
      candidate.preferredKind,
    );
    if (request) requests.push(request);
  }

  if (requests.length > 0) {
    writeLog("info", "open-requests.queued-from-argv", {
      source,
      count: requests.length,
      cwd,
      argv,
    });
  }
  return requests;
}

function enqueueOpenRequest(request) {
  openRequestHistory.push(request);
  if (openRequestHistory.length > openRequestHistoryLimit) {
    openRequestHistory.shift();
  }

  writeLog("info", "open-request.queued", request);
  emitOpenRequest(request);
}

function emitOpenRequest(request) {
  if (!mainWindow || mainWindow.webContents.isDestroyed()) return;
  mainWindow.webContents.send("platform:open-request", request);
}

function flushOpenRequestsToRenderer() {
  if (!mainWindow || mainWindow.webContents.isDestroyed()) return;
  for (const request of openRequestHistory) emitOpenRequest(request);
}

function focusMainWindow() {
  if (!mainWindow || mainWindow.isDestroyed()) return false;
  if (mainWindow.isMinimized()) mainWindow.restore();
  if (!mainWindow.isVisible()) mainWindow.show();
  mainWindow.focus();
  return true;
}

function sanitizeSystemNotificationText(value, label, maxLength, singleLine) {
  if (typeof value !== "string") {
    throw new TypeError(`${label} must be a string`);
  }

  let sanitized = value
    .replace(/\r\n?/g, "\n")
    .replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/g, "")
    .replace(/[\u200b-\u200f\u202a-\u202e\u2060\u2066-\u2069\ufeff]/g, "");
  sanitized = singleLine
    ? sanitized.replace(/\s+/g, " ").trim()
    : sanitized.trim();

  if (!sanitized) throw new Error(`${label} cannot be empty`);
  if (sanitized.length > maxLength) {
    throw new RangeError(`${label} cannot exceed ${maxLength} characters`);
  }
  return sanitized;
}

function normalizeSystemNotificationOptions(options) {
  if (!options || typeof options !== "object" || Array.isArray(options)) {
    throw new TypeError("Notification options must be an object");
  }
  if (options.silent !== undefined && typeof options.silent !== "boolean") {
    throw new TypeError("Notification silent must be a boolean");
  }

  return {
    title: sanitizeSystemNotificationText(
      options.title,
      "Notification title",
      maxSystemNotificationTitleLength,
      true,
    ),
    body: sanitizeSystemNotificationText(
      options.body,
      "Notification body",
      maxSystemNotificationBodyLength,
      false,
    ),
    silent: options.silent === true,
  };
}

function recentWorkspacesPath() {
  return path.join(app.getPath("userData"), recentWorkspacesFile);
}

function toRecentWorkspace(workspaceRoot, lastOpenedAt) {
  return {
    workspaceRoot,
    name: workspaceName(workspaceRoot),
    lastOpenedAt: lastOpenedAt || new Date().toISOString(),
  };
}

function readRecentWorkspaces() {
  try {
    const content = fs.readFileSync(recentWorkspacesPath(), "utf8");
    const parsed = JSON.parse(content);
    if (!Array.isArray(parsed)) return [];

    const seen = new Set();
    const workspaces = [];
    for (const entry of parsed) {
      const rawPath =
        typeof entry === "string"
          ? entry
          : entry?.workspaceRoot || entry?.path || "";
      if (!rawPath) continue;

      try {
        const workspaceRoot = normalizeWorkspaceRoot(rawPath);
        const key = workspaceKey(workspaceRoot);
        if (seen.has(key)) continue;
        seen.add(key);
        workspaces.push(
          toRecentWorkspace(workspaceRoot, entry?.lastOpenedAt || null),
        );
      } catch {
        // Ignore stale or invalid recent entries. They can be re-added by picker.
      }
    }
    return workspaces;
  } catch (error) {
    if (error?.code !== "ENOENT") {
      logConsole("warn", "recent-workspaces.read.failed", { error });
    }
    return [];
  }
}

function writeRecentWorkspaces(workspaces) {
  fs.mkdirSync(path.dirname(recentWorkspacesPath()), { recursive: true });
  fs.writeFileSync(
    recentWorkspacesPath(),
    `${JSON.stringify(workspaces, null, 2)}\n`,
    "utf8",
  );
}

function saveRecentWorkspace(rawPath) {
  const workspaceRoot = normalizeWorkspaceRoot(rawPath);
  const key = workspaceKey(workspaceRoot);
  const current = readRecentWorkspaces().filter(
    (workspace) => workspaceKey(workspace.workspaceRoot) !== key,
  );
  const next = [toRecentWorkspace(workspaceRoot), ...current].slice(
    0,
    maxRecentWorkspaces,
  );
  writeRecentWorkspaces(next);
  return next;
}

function removeRecentWorkspace(rawPath) {
  const workspaceRoot = normalizeComparablePath(rawPath);
  const key = workspaceKey(workspaceRoot);
  const next = readRecentWorkspaces().filter(
    (workspace) => workspaceKey(workspace.workspaceRoot) !== key,
  );
  writeRecentWorkspaces(next);
  return next;
}

async function isBackendHealthy() {
  try {
    const response = await fetch(`${defaultBackendUrl}/health`, {
      headers: { authorization: `Bearer ${backendApiToken}` },
      signal: AbortSignal.timeout(1200),
    });
    const health = response.ok ? await response.json() : null;
    const identityVerified =
      health?.ok === true &&
      health?.service === "opentopia-server" &&
      health?.apiVersion === desktopBackendApiVersion;
    writeLog("info", "backend.health.checked", {
      backend: backendEndpointInfo(),
      ok: response.ok && identityVerified,
      status: response.status,
      identityVerified,
    });
    return response.ok && identityVerified;
  } catch (error) {
    writeLog("warn", "backend.health.failed", {
      backend: backendEndpointInfo(),
      error,
    });
    return false;
  }
}

function canListenOnPort(host, port) {
  return new Promise((resolve) => {
    const probe = net.createServer();
    probe.once("error", () => resolve(false));
    probe.listen({ host, port, exclusive: true }, () => {
      probe.close(() => resolve(true));
    });
  });
}

function findAvailablePort(host) {
  return new Promise((resolve, reject) => {
    const probe = net.createServer();
    probe.once("error", reject);
    probe.listen({ host, port: 0, exclusive: true }, () => {
      const address = probe.address();
      const port = typeof address === "object" && address ? address.port : 0;
      probe.close((error) => {
        if (error) reject(error);
        else if (port > 0) resolve(port);
        else reject(new Error("Could not reserve an available backend port"));
      });
    });
  });
}

async function selectAvailableManagedBackendUrl() {
  if (hasExplicitBackendUrl) return;

  const endpoint = new URL(defaultBackendUrl);
  const host = endpoint.hostname.replace(/^\[|\]$/g, "");
  const port = Number(endpoint.port || "8787");
  if (
    endpoint.protocol !== "http:" ||
    !["127.0.0.1", "::1", "localhost"].includes(host) ||
    !Number.isInteger(port) ||
    port < 1 ||
    port > 65535 ||
    (await canListenOnPort(host, port))
  ) {
    return;
  }

  const availablePort = await findAvailablePort(host);
  const urlHost = host.includes(":") ? `[${host}]` : host;
  defaultBackendUrl = `http://${urlHost}:${availablePort}`;
  writeLog("info", "backend.port.reassigned", {
    previousPort: port,
    selectedPort: availablePort,
    host,
  });
}

function serverBinaryName() {
  return process.platform === "win32"
    ? "opentopia-server.exe"
    : "opentopia-server";
}

function resolvePackagedServerBinary() {
  const bundle = resolvePackagedRuntimeBundle();
  return bundle
    ? { path: bundle.server, exists: true, candidates: [bundle.server] }
    : {
        path: path.join(process.resourcesPath || "", serverBinaryName()),
        exists: false,
        candidates: [
          path.join(process.resourcesPath || "", runtimeManifestName),
        ],
        reason: packagedRuntimeBundleError?.message,
      };
}

function resolvePackagedRuntimeBundle() {
  if (packagedRuntimeBundle || packagedRuntimeBundleError) {
    return packagedRuntimeBundle;
  }
  try {
    packagedRuntimeBundle = loadRuntimeBundle(process.resourcesPath || "");
  } catch (error) {
    packagedRuntimeBundleError = error;
  }
  return packagedRuntimeBundle;
}

function openTopiaWindowsSandboxBinaryName() {
  return process.platform === "win32"
    ? "opentopia-sandbox.exe"
    : "opentopia-sandbox";
}

function cargoTargetDir(repoRoot) {
  return (
    process.env.CARGO_TARGET_DIR ||
    process.env.OPENTOPIA_DEV_CARGO_TARGET_DIR ||
    path.join(repoRoot, "target", "desktop-dev")
  );
}

function resolveOpenTopiaWindowsSandboxBinary(repoRoot) {
  const binaryName = openTopiaWindowsSandboxBinaryName();
  const explicit = process.env.OPENTOPIA_WINDOWS_SANDBOX_BIN;
  const bundle = isDev ? null : resolvePackagedRuntimeBundle();
  const selected =
    explicit ||
    (isDev
      ? path.join(cargoTargetDir(repoRoot), "debug", binaryName)
      : bundle?.sandbox);
  if (!selected || !fs.existsSync(selected)) {
    return {
      path: selected || binaryName,
      exists: false,
      reason:
        packagedRuntimeBundleError?.message ||
        `OpenTopia Windows sandbox helper was not found at the runtime-owned path: ${selected || binaryName}`,
    };
  }
  try {
    const protocol =
      bundle?.sandbox === selected
        ? bundle.sandboxProtocol
        : inspectSandboxProtocol(selected, bundle?.manifest?.sandboxProtocol);
    return { path: selected, exists: true, reason: null, protocol };
  } catch (error) {
    return { path: selected, exists: false, reason: error.message };
  }
}

// A dev backend is compiled ahead of time via `cargo build` and then spawned
// directly, so the binary starts in under a second. Packaged builds launch a
// prebuilt binary and only need a short grace period.
const backendHealthAttempts = isDev ? 60 : 30;

async function waitForBackendHealth(attempts) {
  updateBackendStartupStatus("waiting_for_health");
  for (let i = 0; i < attempts; i += 1) {
    await new Promise((resolve) => setTimeout(resolve, 500));
    if (!backendProcess) {
      writeLog("warn", "backend.spawn.gone", { attempts: i + 1 });
      updateBackendStartupStatus("failed");
      return false;
    }
    if (await isBackendHealthy()) {
      writeLog("info", "backend.spawn.ready", {
        attempts: i + 1,
        backend: backendEndpointInfo(),
      });
      updateBackendStartupStatus("ready");
      void ensureDeploymentLibraryServices();
      return true;
    }
  }
  writeLog("error", "backend.spawn.health-timeout", {
    backend: backendEndpointInfo(),
    attempts,
  });
  updateBackendStartupStatus("failed");
  return false;
}

function devServerBinaryPath(repoRoot) {
  const targetDir = cargoTargetDir(repoRoot);
  const binaryName = `opentopia-server${process.platform === "win32" ? ".exe" : ""}`;
  return path.join(targetDir, "debug", binaryName);
}

async function ensureBackendBuilt(repoRoot) {
  writeLog("info", "backend.build.starting", { repoRoot });
  updateBackendStartupStatus("compiling");
  const env = createBackendEnv(repoRoot);
  return new Promise((resolve, reject) => {
    const packages = ["build", "-p", "opentopia-server"];
    if (process.platform === "win32") {
      packages.push("-p", "opentopia-windows-sandbox");
    }
    const child = spawn("cargo", packages, {
      cwd: repoRoot,
      env,
      stdio: ["ignore", "pipe", "pipe"],
      windowsHide: true,
    });
    let stderrTail = "";
    child.stderr?.on("data", (chunk) => {
      const output = chunk.toString();
      stderrTail = `${stderrTail}${output}`.slice(-2000);
      for (const rawLine of output.split(/\r?\n/)) {
        const line = rawLine.replace(/\u001B\[[0-?]*[ -/]*[@-~]/g, "");
        const compiling = line.match(/^\s*(?:Compiling|Checking)\s+([^\s]+)/);
        if (compiling) {
          updateBackendStartupStatus("compiling", {
            detail: compiling[1],
          });
        }
      }
    });
    child.on("exit", (code) => {
      if (code === 0) {
        writeLog("info", "backend.build.completed");
        resolve();
      } else {
        writeLog("error", "backend.build.failed", { code, stderr: stderrTail });
        updateBackendStartupStatus("failed", { detail: "编译未能完成" });
        reject(new Error(`cargo build exited with code ${code}`));
      }
    });
    child.on("error", (error) => {
      writeLog("error", "backend.build.error", { error: error.message });
      updateBackendStartupStatus("failed", { detail: "编译未能开始" });
      reject(error);
    });
  });
}

async function startBackendIfNeeded({
  waitForHealth = true,
  attempts = backendHealthAttempts,
} = {}) {
  updateBackendStartupStatus("checking", { resetElapsed: true });
  if (await isBackendHealthy()) {
    updateBackendStartupStatus("ready");
    void ensureDeploymentLibraryServices();
    return;
  }

  try {
    await selectAvailableManagedBackendUrl();
  } catch (error) {
    logConsole("warn", "backend.port.selection.failed", { error });
  }

  const repoRoot = path.resolve(__dirname, "..", "..", "..");
  const packagedServer = resolvePackagedServerBinary();
  if (!isDev && !packagedServer.exists) {
    writeLog("error", "backend.packaged-server.missing", {
      backend: backendEndpointInfo(),
      packagedServer: packagedServer.path,
      packagedServerCandidates: packagedServer.candidates,
    });
    updateBackendStartupStatus("failed", { detail: "本地服务文件缺失" });
    return;
  }

  const endpoint = new URL(defaultBackendUrl);
  const endpointHost = endpoint.hostname.replace(/^\[|\]$/g, "");
  if (
    endpoint.protocol !== "http:" ||
    !["127.0.0.1", "::1", "localhost"].includes(endpointHost)
  ) {
    updateBackendStartupStatus("failed", { detail: "本地服务地址无效" });
    throw new Error(
      "OPENTOPIA_SERVER_URL must use HTTP on a loopback host for the local desktop server.",
    );
  }
  const serverArgs = [
    "--host",
    endpointHost === "localhost" ? "127.0.0.1" : endpointHost,
    "--port",
    endpoint.port || "8787",
  ];

  let command, args, cwd;
  if (isDev) {
    await ensureBackendBuilt(repoRoot);
    command = devServerBinaryPath(repoRoot);
    args = serverArgs;
    cwd = undefined;
  } else {
    command = packagedServer.path;
    args = serverArgs;
    cwd = undefined;
  }

  try {
    updateBackendStartupStatus("starting");
    writeLog("info", "backend.spawn.starting", {
      backend: backendEndpointInfo(),
      command,
      args,
      cwd,
      packagedServer: packagedServer.path,
      packagedServerCandidates: packagedServer.candidates,
      packagedServerExists: packagedServer.exists,
      isDev,
    });

    const spawnedBackend = spawn(command, args, {
      cwd,
      env: createBackendEnv(repoRoot),
      stdio: ["ignore", "pipe", "pipe"],
      windowsHide: true,
    });
    backendProcess = spawnedBackend;

    writeLog("info", "backend.spawn.started", {
      pid: backendProcess.pid,
      command,
      args,
      cwd,
    });

    spawnedBackend.stdout?.on("data", (chunk) =>
      logConsole("info", "backend.stdout", {
        chunk: chunk.toString(),
      }),
    );
    spawnedBackend.stderr?.on("data", (chunk) =>
      logConsole("error", "backend.stderr", {
        chunk: chunk.toString(),
      }),
    );
    spawnedBackend.on("exit", (code) => {
      writeLog("info", "backend.spawn.exited", { code });
      if (backendProcess === spawnedBackend) backendProcess = null;
      updateBackendStartupStatus("failed", { detail: "本地服务已退出" });
    });

    // The renderer keeps probing /health on its own, so the window does not have
    // to wait out a long build before it can be shown.
    const readiness = waitForBackendHealth(attempts).catch((error) => {
      logConsole("warn", "backend.health.wait.failed", { error });
      return false;
    });
    if (waitForHealth) await readiness;
  } catch (error) {
    logConsole("error", "backend.spawn.failed", { error });
    updateBackendStartupStatus("failed", { detail: "本地服务未能启动" });
  }
}

// In dev the tracked child is `cargo`, which spawns the real server as a
// grandchild. Killing only `cargo` leaves that server alive and squatting the
// backend port, so the next launch has to fall back to a random port. Quit is
// synchronous, hence spawnSync rather than the async stopManagedBackend path.
function killBackendProcessTree() {
  const child = backendProcess;
  if (!child?.pid) return;
  backendProcess = null;
  if (process.platform === "win32") {
    try {
      spawnSync("taskkill", ["/pid", String(child.pid), "/t", "/f"], {
        windowsHide: true,
        stdio: "ignore",
      });
      return;
    } catch (error) {
      logConsole("warn", "backend.kill.taskkill.failed", { error });
    }
  }
  child.kill();
}

async function requestBackendShutdownPreparation() {
  if (!backendProcess) return null;
  const response = await fetch(
    `${defaultBackendUrl}/api/runtime/prepare-shutdown`,
    {
      method: "POST",
      headers: { authorization: `Bearer ${backendApiToken}` },
      signal: AbortSignal.timeout(7_000),
    },
  );
  if (!response.ok) {
    throw new Error(
      `Backend shutdown preparation failed with HTTP ${response.status}`,
    );
  }
  const result = await response.json();
  writeLog(
    result.remaining > 0 ? "warn" : "info",
    "backend.shutdown.prepared",
    result,
  );
  return result;
}

async function allSettledWithin(promises, timeoutMs) {
  let timer = null;
  try {
    return await Promise.race([
      Promise.allSettled(promises),
      new Promise((resolve) => {
        timer = setTimeout(() => resolve(null), timeoutMs);
      }),
    ]);
  } finally {
    if (timer) clearTimeout(timer);
  }
}

async function stopManagedBackend() {
  const child = backendProcess;
  if (!child) return;

  try {
    await requestBackendShutdownPreparation();
  } catch (error) {
    logConsole("warn", "backend.shutdown.prepare.failed", { error });
  }
  if (
    backendProcess !== child ||
    child.exitCode !== null ||
    child.signalCode !== null
  ) {
    return;
  }

  await new Promise((resolve, reject) => {
    let settled = false;
    const finish = (error) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      if (error) reject(error);
      else resolve();
    };
    const timer = setTimeout(
      () => finish(new Error("Timed out while stopping the local backend")),
      10_000,
    );
    child.once("exit", () => finish());
    child.once("error", finish);

    if (process.platform === "win32") {
      const killer = spawn(
        "taskkill",
        ["/pid", String(child.pid), "/t", "/f"],
        { windowsHide: true, stdio: "ignore" },
      );
      killer.once("error", finish);
    } else if (!child.kill("SIGTERM")) {
      finish(new Error("The local backend process could not be stopped"));
    }
  });
}

/**
 * Restarts the backend so it picks up a credential change.
 *
 * The secret is already durable on disk by the time this runs, so a failed
 * restart must not be reported to the renderer as a failed save — it is a
 * separate, recoverable condition. Callers get a structured outcome instead of
 * an exception, and the renderer surfaces the two independently.
 */
async function restartBackendAfterSecretChange(context) {
  try {
    await restartManagedBackend();
    return { restarted: true, error: null };
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    writeLog("warn", "secrets.backend-restart.failed", {
      ...context,
      error: message,
    });
    return { restarted: false, error: message };
  }
}

async function restartManagedBackend() {
  if (!backendProcess) {
    throw new Error("The local backend is not managed by this desktop process");
  }
  await stopManagedBackend();
  // Bounded so a restart triggered from the UI cannot hang on a long rebuild;
  // the renderer keeps probing if the build outlasts this window.
  await startBackendIfNeeded({ attempts: 30 });
  if (!(await isBackendHealthy())) {
    throw new Error("The local backend did not become ready after restart");
  }
}

function createMainWindow() {
  writeLog("info", "window.create.starting", {
    pendingOpenRequests: openRequestHistory.length,
  });

  const createdWindow = new BrowserWindow({
    width: 1440,
    height: 900,
    minWidth: 1080,
    minHeight: 720,
    title: isDev ? "OpenTopia Dev" : "OpenTopia",
    backgroundColor: windowBackgroundColors.light,
    show: false,
    ...(process.platform === "win32"
      ? {
          titleBarStyle: "hidden",
          titleBarOverlay: titleBarOverlayFor("light"),
        }
      : {}),
    webPreferences: {
      preload: path.join(__dirname, "preload.cjs"),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: false,
    },
  });

  appWindows.add(createdWindow);
  mainWindow = createdWindow;
  desktopBrowserHost?.attachWindow(createdWindow);

  createdWindow.on("focus", () => {
    if (createdWindow.isDestroyed() || mainWindow === createdWindow) return;
    mainWindow = createdWindow;
    desktopBrowserHost?.attachWindow(createdWindow);
  });

  if (isDev) {
    createdWindow.on("page-title-updated", (event) => {
      event.preventDefault();
      createdWindow.setTitle("OpenTopia Dev");
    });
  }

  createdWindow.once("ready-to-show", () => {
    createdWindow.show();
    createdWindow.focus();
    flushOpenRequestsToRenderer();
  });

  createdWindow.webContents.once("did-finish-load", () => {
    writeLog("info", "window.load.finished", {
      url: createdWindow.webContents.getURL(),
      pendingOpenRequests: openRequestHistory.length,
    });
    publishBackendStartupStatus();
    flushOpenRequestsToRenderer();
  });

  createdWindow.on("closed", () => {
    writeLog("info", "window.closed");
    appWindows.delete(createdWindow);
    if (mainWindow !== createdWindow) return;
    mainWindow = [...appWindows].at(-1) ?? null;
    if (mainWindow) desktopBrowserHost?.attachWindow(mainWindow);
  });

  if (isDev) {
    createdWindow.loadURL(
      process.env.VITE_DEV_SERVER_URL || "http://127.0.0.1:5173",
    );
    createdWindow.webContents.openDevTools({ mode: "detach" });
  } else {
    createdWindow.loadFile(path.join(__dirname, "..", "dist", "index.html"));
  }

  if (appWindows.size === 1) {
    updater.setupAutoUpdater(createdWindow);
  }
  if (!isDev && appWindows.size === 1) {
    updater.checkForUpdates();
  }

  return createdWindow;
}

function resolveRepoRoot() {
  return path.resolve(__dirname, "..", "..", "..");
}

function getSagServiceManager() {
  if (sagServiceManager) return sagServiceManager;
  const repoRoot = resolveRepoRoot();
  const runtimeEnv = { ...process.env };
  importEnvFile(runtimeEnv, resolveOpenTopiaEnvFile(repoRoot));
  sagServiceManager = createSagServiceManager({
    endpoint: runtimeEnv.OPENTOPIA_SAG_URL || DEFAULT_SAG_URL,
    env: runtimeEnv,
    isPackaged: app.isPackaged,
    repoRoot,
    resourcesPath: process.resourcesPath,
    logger: (level, event, metadata) => logConsole(level, event, metadata),
  });
  return sagServiceManager;
}

function getGraphRagServiceManager() {
  if (graphRagServiceManager) return graphRagServiceManager;
  const repoRoot = resolveRepoRoot();
  const runtimeEnv = { ...process.env };
  importEnvFile(runtimeEnv, resolveOpenTopiaEnvFile(repoRoot));
  graphRagServiceManager = createGraphRagServiceManager({
    endpoint: runtimeEnv.OPENTOPIA_GRAPH_RAG_URL || DEFAULT_GRAPH_RAG_URL,
    env: runtimeEnv,
    isPackaged: app.isPackaged,
    repoRoot,
    resourcesPath: process.resourcesPath,
    logger: (level, event, metadata) => logConsole(level, event, metadata),
  });
  return graphRagServiceManager;
}

function getLibraryProviderServiceManager(provider) {
  if (provider === "sag") return getSagServiceManager();
  if (provider === "graph-rag") return getGraphRagServiceManager();
  throw new Error(`未知的资料库后端：${provider}`);
}

async function ensureDeploymentLibraryServices() {
  try {
    const providers = await autostartDeploymentLibraryServices({
      backendUrl: defaultBackendUrl,
      apiToken: backendApiToken,
      ensureProvider: (provider) =>
        getLibraryProviderServiceManager(provider).ensureReady(),
    });
    if (providers.length > 0) {
      writeLog("info", "library.provider.autostart.ready", { providers });
    }
  } catch (error) {
    logConsole("warn", "library.provider.autostart.failed", { error });
  }
}

function listSecretSources() {
  const backendEnv = createBackendEnv(resolveRepoRoot(), {
    includeKeyring: false,
  });
  const envSources = providerSecretEnvNames.map((envName) => ({
    id: `env:${envName}`,
    kind: "environment",
    label: envName,
    envName,
    configured: Boolean(backendEnv[envName]),
    readableByRenderer: false,
    storesValue: false,
    status: "available",
  }));
  const keyring = keyringMetadata();
  const providerKeySources = Object.keys(readSecretStore().secrets)
    .filter((key) => key.startsWith(providerSecretStoragePrefix))
    .flatMap((key) => {
      try {
        const providerId = key.slice(providerSecretStoragePrefix.length);
        const metadata = providerKeyringMetadata(providerId);
        return [
          {
            id: metadata.providerApiKeySourceId,
            kind: "keyring",
            label: `Provider API key (${providerId})`,
            configured: metadata.providerApiKeyConfigured,
            readableByRenderer: false,
            storesValue: true,
            status: metadata.status,
            available: metadata.available,
            storageBackend: metadata.storageBackend,
            storagePath: metadata.storagePath,
            envTarget: metadata.envTarget,
            providerId,
          },
        ];
      } catch {
        return [];
      }
    });
  const activeProviderKeySource =
    (keyring.available && keyring.providerApiKeyConfigured
      ? keyringProviderApiKeySourceId
      : null) ||
    envSources.find(
      (source) => source.envName === "OPENTOPIA_API_KEY" && source.configured,
    )?.id ||
    envSources.find((source) => source.configured)?.id ||
    null;

  return {
    activeProviderKeySource,
    keyring,
    sources: [
      ...envSources,
      {
        id: keyringProviderApiKeySourceId,
        kind: "keyring",
        label: "Provider API key",
        envName: keyring.envTarget,
        configured: keyring.providerApiKeyConfigured,
        readableByRenderer: false,
        storesValue: true,
        status: keyring.status,
        available: keyring.available,
        storageBackend: keyring.storageBackend,
        storagePath: keyring.storagePath,
        envTarget: keyring.envTarget,
      },
      ...providerKeySources,
    ],
    notes: [
      "Renderer receives metadata only. Secret values stay in env/keyring-capable main process paths.",
      "The keyring storage path is metadata only and never contains the secret value.",
    ],
  };
}

function registerOpenTopiaProtocolClient() {
  try {
    if (isDev && process.env.OPENTOPIA_REGISTER_PROTOCOL !== "1") {
      writeLog("info", "protocol.registration.skipped", {
        scheme: openTopiaProtocol,
        reason: "dev opt-in via OPENTOPIA_REGISTER_PROTOCOL=1",
      });
      return false;
    }

    protocolClientRegistered =
      process.defaultApp && process.argv.length >= 2
        ? app.setAsDefaultProtocolClient(openTopiaProtocol, process.execPath, [
            path.resolve(process.argv[1]),
          ])
        : app.setAsDefaultProtocolClient(openTopiaProtocol);

    writeLog("info", "protocol.registration.completed", {
      scheme: openTopiaProtocol,
      registered: protocolClientRegistered,
    });
    return protocolClientRegistered;
  } catch (error) {
    protocolClientRegistered = false;
    logConsole("warn", "protocol.registration.failed", { error });
    return false;
  }
}

function registerIpc() {
  ipcMain.handle("platform:new-window", (event) => {
    const owner = BrowserWindow.fromWebContents(event.sender);
    if (!owner || !appWindows.has(owner)) {
      throw new Error(
        "New windows can be opened only by an application window",
      );
    }
    createMainWindow();
    return true;
  });

  const assertMainRenderer = (event) => {
    const owner = BrowserWindow.fromWebContents(event.sender);
    if (
      !owner ||
      !appWindows.has(owner) ||
      event.senderFrame !== event.sender.mainFrame
    ) {
      throw new Error(
        "This IPC action is restricted to an application main frame.",
      );
    }
  };
  ipcMain.handle("platform:show-tool-menu", (event, request) => {
    assertMainRenderer(event);
    const owner = BrowserWindow.fromWebContents(event.sender);
    if (!owner || owner.isDestroyed()) {
      throw new Error("The application window is unavailable.");
    }
    const options = normalizeDesktopToolMenuRequest(request);
    return new Promise((resolve) => {
      let selectedAction = null;
      const menu = Menu.buildFromTemplate(
        createDesktopToolMenuTemplate(options, (action) => {
          selectedAction = action;
        }),
      );
      const popupOptions = {
        window: owner,
        callback: () => resolve(selectedAction),
      };
      if (options.x !== undefined && options.y !== undefined) {
        popupOptions.x = options.x;
        popupOptions.y = options.y;
      }
      menu.popup(popupOptions);
    });
  });
  ipcMain.on("backend-event-stream:open", (event, request) => {
    try {
      assertMainRenderer(event);
      backendEventStreamManager.open(event.sender, request);
    } catch (error) {
      backendEventStreamManager.reject(event.sender, request, error);
    }
  });
  ipcMain.on("backend-event-stream:close", (event, streamId) => {
    try {
      assertMainRenderer(event);
      backendEventStreamManager.close(event.sender, streamId);
    } catch {
      // A destroyed or navigated renderer already lost its stream consumer.
    }
  });
  ipcMain.handle("chrome-bridge:start-pairing", (event, sessionId) => {
    assertMainRenderer(event);
    if (!chromeBridge) throw new Error("Chrome bridge is unavailable.");
    return chromeBridge.startPairing(sessionId);
  });
  ipcMain.handle("chrome-bridge:get-status", (event, sessionId) => {
    assertMainRenderer(event);
    if (!chromeBridge) throw new Error("Chrome bridge is unavailable.");
    return chromeBridge.getStatus(sessionId);
  });
  ipcMain.handle("chrome-bridge:disconnect", (event, sessionId) => {
    assertMainRenderer(event);
    if (!chromeBridge) throw new Error("Chrome bridge is unavailable.");
    return chromeBridge.disconnect(sessionId);
  });
  ipcMain.handle("chrome-bridge:action", (event, sessionId, action, value) => {
    assertMainRenderer(event);
    if (!chromeBridge) throw new Error("Chrome bridge is unavailable.");
    return chromeBridge.runUserAction(sessionId, action, value);
  });

  ipcMain.handle("platform:close-window", (event) => {
    const owner = BrowserWindow.fromWebContents(event.sender);
    if (!owner || !appWindows.has(owner)) {
      throw new Error("Only an application window can close itself");
    }
    owner.close();
    return true;
  });

  ipcMain.handle("platform:quit", (event) => {
    const owner = BrowserWindow.fromWebContents(event.sender);
    if (!owner || !appWindows.has(owner)) {
      throw new Error("Only an application window can quit the application");
    }
    app.quit();
    return true;
  });

  ipcMain.handle("platform:get-info", () => ({
    platform: "desktop",
    os: process.platform,
    arch: process.arch,
    versions: process.versions,
    backendUrl: defaultBackendUrl,
    apiToken: backendApiToken,
    keyring: keyringMetadata(),
    paths: {
      userData: app.getPath("userData"),
      logs: getLogPaths().logsDirPath,
      crashLogs: getLogPaths().crashLogsDirPath,
    },
    protocol: {
      scheme: openTopiaProtocol,
      registered: protocolClientRegistered,
    },
  }));

  ipcMain.handle("backend:get-startup-status", (event) => {
    assertMainRenderer(event);
    return backendStartupStatusSnapshot();
  });

  ipcMain.handle("library:sag:ensure-ready", (event) => {
    assertMainRenderer(event);
    return getSagServiceManager().ensureReady();
  });

  ipcMain.handle("library:provider:ensure-ready", (event, provider) => {
    assertMainRenderer(event);
    return getLibraryProviderServiceManager(provider).ensureReady();
  });

  ipcMain.handle("platform:get-open-requests", () =>
    openRequestHistory.map((request) => ({ ...request })),
  );

  // Called by the renderer every time the resolved appearance changes.
  ipcMain.handle("platform:set-theme", (_event, theme) => {
    const resolved = theme === "dark" ? "dark" : "light";
    if (!mainWindow || mainWindow.isDestroyed()) return false;
    // Repainting the window background too avoids a pale flash on resize.
    mainWindow.setBackgroundColor(windowBackgroundColors[resolved]);
    if (process.platform === "win32") {
      try {
        mainWindow.setTitleBarOverlay(titleBarOverlayFor(resolved));
      } catch (error) {
        writeLog("warn", "titlebar.overlay.failed", { error: String(error) });
        return false;
      }
    }
    return true;
  });

  ipcMain.handle("platform:show-system-notification", (event, options) => {
    if (!mainWindow || event.sender !== mainWindow.webContents) {
      throw new Error(
        "System notifications are available only to the main window",
      );
    }

    const notificationOptions = normalizeSystemNotificationOptions(options);
    if (!Notification.isSupported()) return false;

    const notification = new Notification(notificationOptions);
    activeSystemNotifications.add(notification);
    if (activeSystemNotifications.size > maxRetainedSystemNotifications) {
      activeSystemNotifications.delete(
        activeSystemNotifications.values().next().value,
      );
    }
    notification.once("click", () => {
      activeSystemNotifications.delete(notification);
      if (!focusMainWindow() && app.isReady()) createMainWindow();
    });
    notification.once("failed", (_event, error) => {
      activeSystemNotifications.delete(notification);
      logConsole("warn", "notification.show.failed", { error });
    });
    notification.show();
    return true;
  });

  ipcMain.handle("platform:write-clipboard-image", (event, bytes) => {
    if (!mainWindow || event.sender !== mainWindow.webContents) {
      throw new Error(
        "Clipboard image writes are available only to the main window",
      );
    }
    if (!ArrayBuffer.isView(bytes) || bytes.byteLength === 0) {
      throw new TypeError(
        "Clipboard image bytes must be a non-empty typed array",
      );
    }
    const buffer = Buffer.from(
      bytes.buffer,
      bytes.byteOffset,
      bytes.byteLength,
    );
    const image = nativeImage.createFromBuffer(buffer);
    if (image.isEmpty()) throw new Error("Clipboard image data is invalid");
    clipboard.writeImage(image);
    return true;
  });

  ipcMain.handle("secrets:list-sources", () => listSecretSources());

  ipcMain.handle("secrets:set", async (_event, key, value) => {
    normalizeProviderSecretKey(key);
    const metadata = setProviderApiKeySecret(value);
    writeLog("info", "secrets.provider.set", {
      sourceId: keyringProviderApiKeySourceId,
      configured: metadata.providerApiKeyConfigured,
      status: metadata.status,
    });
    const backendRestart = await restartBackendAfterSecretChange({
      operation: "set-secret",
    });
    return { ...listSecretSources(), backendRestart };
  });

  ipcMain.handle("secrets:get-provider-key-metadata", (_event, providerId) =>
    providerKeyringMetadata(providerId),
  );

  ipcMain.handle(
    "secrets:set-provider-key",
    async (_event, providerId, value) => {
      const metadata = setProviderKeyringSecret(providerId, value);
      writeLog("info", "secrets.provider-profile.set", {
        providerId: metadata.providerId,
        sourceId: metadata.providerApiKeySourceId,
        configured: metadata.providerApiKeyConfigured,
        status: metadata.status,
      });
      const backendRestart = await restartBackendAfterSecretChange({
        operation: "set-provider-key",
        providerId: metadata.providerId,
      });
      return { ...metadata, backendRestart };
    },
  );

  ipcMain.handle("secrets:delete-provider-key", async (_event, providerId) => {
    const metadata = deleteProviderKeyringSecret(providerId);
    writeLog("info", "secrets.provider-profile.delete", {
      providerId: metadata.providerId,
      sourceId: metadata.providerApiKeySourceId,
      configured: metadata.providerApiKeyConfigured,
      status: metadata.status,
    });
    const backendRestart = await restartBackendAfterSecretChange({
      operation: "delete-provider-key",
      providerId: metadata.providerId,
    });
    return { ...metadata, backendRestart };
  });

  ipcMain.handle("secrets:delete", async (_event, key) => {
    normalizeProviderSecretKey(key);
    const metadata = deleteProviderApiKeySecret();
    writeLog("info", "secrets.provider.delete", {
      sourceId: keyringProviderApiKeySourceId,
      configured: metadata.providerApiKeyConfigured,
      status: metadata.status,
    });
    const backendRestart = await restartBackendAfterSecretChange({
      operation: "delete-secret",
    });
    return { ...listSecretSources(), backendRestart };
  });

  ipcMain.handle("logs:list", async () => {
    const { logsDirPath } = getLogPaths();
    if (!logsDirPath) return [];
    try {
      const entries = fs.readdirSync(logsDirPath, { withFileTypes: true });
      const files = entries
        .filter((entry) => entry.isFile() && entry.name.endsWith(".jsonl"))
        .map((entry) => {
          const filePath = path.join(logsDirPath, entry.name);
          const stat = fs.statSync(filePath);
          return {
            name: entry.name,
            path: filePath,
            size: stat.size,
            modifiedAt: stat.mtime.toISOString(),
          };
        })
        .sort((a, b) => b.modifiedAt.localeCompare(a.modifiedAt));
      return files;
    } catch {
      return [];
    }
  });

  ipcMain.handle("logs:read", async (_event, filePath, offset, limit) => {
    const { logsDirPath } = getLogPaths();
    const resolvedPath = path.resolve(filePath);
    if (!resolvedPath.startsWith(path.resolve(logsDirPath || ""))) {
      throw new Error("Access denied: log file path is outside logs directory");
    }
    try {
      const content = fs.readFileSync(resolvedPath, "utf8");
      const allLines = content.split("\n");
      const start = offset || 0;
      const count = limit || 100;
      const lines = allLines.slice(start, start + count);
      return { lines, total: allLines.length };
    } catch {
      return { lines: [], total: 0 };
    }
  });

  ipcMain.on("logs:conversation-render-trace", (event, trace) => {
    if (!mainWindow || event.sender !== mainWindow.webContents) return;
    if (!trace || typeof trace !== "object" || Array.isArray(trace)) return;
    const stage = ["received", "committed", "painted"].includes(trace.stage)
      ? trace.stage
      : "unknown";
    writeLog("info", `conversation.render.${stage}`, trace);
  });

  ipcMain.on("logs:conversation-send-trace", (event, trace) => {
    if (!mainWindow || event.sender !== mainWindow.webContents) return;
    if (!trace || typeof trace !== "object" || Array.isArray(trace)) return;
    const stage = [
      "controller_started",
      "state_dispatched",
      "fetch_started",
      "response_headers",
      "response_parsed",
      "state_confirmed",
      "failed",
    ].includes(trace.stage)
      ? trace.stage
      : "unknown";
    writeLog("info", `conversation.send.${stage}`, trace);
  });

  ipcMain.handle("platform:open-external", async (_event, rawUrl) => {
    const url = new URL(rawUrl);
    if (!["http:", "https:", "mailto:"].includes(url.protocol)) {
      throw new Error(`Blocked external URL protocol: ${url.protocol}`);
    }
    await shell.openExternal(url.toString());
  });

  ipcMain.handle("platform:open-path", async (_event, rawPath) => {
    const targetPath = normalizeExistingPath(rawPath);
    const error = await shell.openPath(targetPath);
    if (error) throw new Error(error);
    return { path: targetPath };
  });

  ipcMain.handle("platform:file-link-action", async (event, request) => {
    if (!request || typeof request !== "object" || Array.isArray(request)) {
      throw new Error("File action request must be an object.");
    }
    const filePath = normalizeExistingFile(request.path);

    if (request.action === "open-default") {
      const error = await shell.openPath(filePath);
      if (error) throw new Error(error);
      return { path: filePath };
    }
    if (request.action === "open-vscode") {
      await shell.openExternal(visualStudioCodeFileUrl(filePath, request.line));
      return { path: filePath };
    }
    if (request.action === "open-with") {
      await showOpenWithDialog(userVisibleWindowsPath(filePath));
      return { path: filePath };
    }
    if (request.action === "save-as") {
      const owner = BrowserWindow.fromWebContents(event.sender) || mainWindow;
      const dialogOptions = {
        title: "另存文件",
        defaultPath: userVisibleWindowsPath(filePath),
      };
      const result = owner
        ? await dialog.showSaveDialog(owner, dialogOptions)
        : await dialog.showSaveDialog(dialogOptions);
      if (result.canceled || !result.filePath) return { canceled: true };
      const destination = path.resolve(result.filePath);
      if (destination !== path.resolve(filePath)) {
        fs.copyFileSync(filePath, destination);
      }
      return { canceled: false, path: destination };
    }
    if (request.action === "reveal") {
      shell.showItemInFolder(filePath);
      return { path: filePath };
    }

    throw new Error(`Unsupported file action: ${request.action}`);
  });

  ipcMain.handle("workspace:select", async (event, options = {}) => {
    let defaultPath;
    if (typeof options?.defaultPath === "string" && options.defaultPath) {
      try {
        defaultPath = normalizeWorkspaceRoot(options.defaultPath);
      } catch {
        defaultPath = undefined;
      }
    }

    const owner = BrowserWindow.fromWebContents(event.sender) || mainWindow;
    const dialogOptions = {
      title: "Open Workspace",
      defaultPath,
      properties: ["openDirectory", "createDirectory"],
    };
    const result = owner
      ? await dialog.showOpenDialog(owner, dialogOptions)
      : await dialog.showOpenDialog(dialogOptions);

    if (result.canceled || result.filePaths.length === 0) {
      return { canceled: true };
    }

    const workspaceRoot = normalizeWorkspaceRoot(result.filePaths[0]);
    const recentWorkspaces = saveRecentWorkspace(workspaceRoot);
    return {
      canceled: false,
      workspaceRoot,
      workspace: recentWorkspaces[0],
      recentWorkspaces,
    };
  });

  ipcMain.handle("context:select-files", async (event, options = {}) => {
    let defaultPath;
    if (typeof options?.defaultPath === "string" && options.defaultPath) {
      try {
        defaultPath = normalizeComparablePath(options.defaultPath);
      } catch {
        defaultPath = undefined;
      }
    }

    const owner = BrowserWindow.fromWebContents(event.sender) || mainWindow;
    const dialogOptions = {
      title: "Add context files",
      defaultPath,
      properties: ["openFile", "multiSelections"],
      filters: [
        {
          name: "Supported context files",
          extensions: supportedContextFileExtensions,
        },
      ],
    };
    const result = owner
      ? await dialog.showOpenDialog(owner, dialogOptions)
      : await dialog.showOpenDialog(dialogOptions);

    if (result.canceled || result.filePaths.length === 0) {
      return { canceled: true, files: [] };
    }
    if (result.filePaths.length > maxContextSourceFiles) {
      throw new Error(
        `Select at most ${maxContextSourceFiles} context files at once.`,
      );
    }
    return {
      canceled: false,
      files: result.filePaths.map(contextSourceMetadata),
    };
  });

  ipcMain.handle("context:add-dropped-files", async (_event, filePaths) => {
    if (
      !Array.isArray(filePaths) ||
      filePaths.some((filePath) => typeof filePath !== "string")
    ) {
      throw new Error("Dropped files must be provided as file paths.");
    }
    if (filePaths.length === 0) {
      return { canceled: true, files: [] };
    }
    if (filePaths.length > maxContextSourceFiles) {
      throw new Error(
        `Drop at most ${maxContextSourceFiles} context files at once.`,
      );
    }
    return {
      canceled: false,
      files: filePaths.map(contextSourceMetadata),
    };
  });

  ipcMain.handle("plugins:select-directory", async (event, options = {}) => {
    let defaultPath;
    if (typeof options?.defaultPath === "string" && options.defaultPath) {
      try {
        defaultPath = normalizeComparablePath(options.defaultPath);
      } catch {
        defaultPath = undefined;
      }
    }

    const owner = BrowserWindow.fromWebContents(event.sender) || mainWindow;
    const dialogOptions = {
      title: "Install local plugin",
      defaultPath,
      properties: ["openDirectory"],
      message: "Select a folder containing .codex-plugin/plugin.json",
    };
    const result = owner
      ? await dialog.showOpenDialog(owner, dialogOptions)
      : await dialog.showOpenDialog(dialogOptions);
    if (result.canceled || result.filePaths.length === 0) {
      return { canceled: true };
    }
    return {
      canceled: false,
      path: normalizeComparablePath(result.filePaths[0]),
    };
  });

  ipcMain.handle("workspace:get-recent", () => readRecentWorkspaces());

  ipcMain.handle("workspace:save-recent", (_event, rawPath) =>
    saveRecentWorkspace(rawPath),
  );

  ipcMain.handle("workspace:remove-recent", (_event, rawPath) =>
    removeRecentWorkspace(rawPath),
  );

  ipcMain.handle("workspace:clear-recent", () => {
    writeRecentWorkspaces([]);
    return [];
  });
}

ensureLoggingInitialized();

const singleInstance = app.requestSingleInstanceLock();
if (!singleInstance) {
  app.quit();
} else {
  app.on("second-instance", (_event, commandLine, workingDirectory) => {
    queueOpenRequestsFromArgv("second-instance", commandLine, workingDirectory);
    if (!mainWindow) return;
    if (mainWindow.isMinimized()) mainWindow.restore();
    mainWindow.focus();
  });

  app.on("open-url", (event, rawUrl) => {
    event.preventDefault();
    queueOpenRequestFromValue("open-url", rawUrl, process.cwd());
    focusMainWindow();
  });

  app.whenReady().then(async () => {
    queueOpenRequestsFromArgv("startup", process.argv, process.cwd());
    registerOpenTopiaProtocolClient();
    desktopBrowserHost = createDesktopBrowserHost({
      app,
      Menu,
      WebContentsView,
      clipboard,
      dialog,
      nativeImage,
      getMainWindow: () => mainWindow,
      logger: (level, event, metadata) => logConsole(level, event, metadata),
    });
    chromeBridge = createChromeBridge({
      extensionId: process.env.OPENTOPIA_CHROME_EXTENSION_ID || undefined,
      logger: (level, event, metadata) => logConsole(level, event, metadata),
      onStateChanged: (state) => {
        if (
          mainWindow &&
          !mainWindow.isDestroyed() &&
          !mainWindow.webContents.isDestroyed()
        ) {
          mainWindow.webContents.send("chrome-bridge:state", state);
        }
      },
    });
    try {
      desktopBrowserBroker = await desktopBrowserHost.startBroker();
    } catch (error) {
      logConsole("error", "browser.broker.start.failed", { error });
    }
    try {
      chromeBridgeBackend = await chromeBridge.start();
    } catch (error) {
      logConsole("error", "chrome.bridge.start.failed", { error });
    }
    registerIpc();
    desktopBrowserHost.registerIpc(ipcMain);
    // Show the window immediately so the user sees something while cargo
    // builds the backend binary on first MSVC launch (~3 min cold build).
    createMainWindow();
    startBackendIfNeeded({ waitForHealth: false }).catch((error) => {
      logConsole("error", "backend.init.failed", { error });
    });

    app.on("activate", () => {
      if (BrowserWindow.getAllWindows().length === 0) createMainWindow();
    });
  });
}

app.on("window-all-closed", () => {
  if (process.platform !== "darwin") app.quit();
});

app.on("before-quit", (event) => {
  if (appQuitPrepared) {
    flushLogsSync();
    return;
  }
  event.preventDefault();
  if (appQuitPreparation) return;

  appQuitPreparation = (async () => {
    try {
      await stopManagedBackend();
    } catch (error) {
      logConsole("warn", "backend.shutdown.failed", { error });
      killBackendProcessTree();
    }
    const closingServices = [
      desktopBrowserHost?.close(),
      chromeBridge?.close(),
    ].filter(Boolean);
    const results = await allSettledWithin(closingServices, 3_000);
    if (results === null) {
      writeLog("warn", "desktop.service.close.timed-out");
    } else {
      for (const result of results) {
        if (result.status === "rejected") {
          logConsole("warn", "desktop.service.close.failed", {
            error: result.reason,
          });
        }
      }
    }
    sagServiceManager?.stopSync();
    graphRagServiceManager?.stopSync();
    backendEventStreamManager.closeAll();
  })().finally(() => {
    appQuitPrepared = true;
    flushLogsSync();
    app.quit();
  });
});
