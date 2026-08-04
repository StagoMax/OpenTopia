const {
  app,
  BrowserWindow,
  WebContentsView,
  dialog,
  ipcMain,
  Notification,
  safeStorage,
  shell,
} = require("electron");
const path = require("node:path");
const { URL, fileURLToPath } = require("node:url");
const { spawn, spawnSync } = require("node:child_process");
const crypto = require("node:crypto");
const fs = require("node:fs");
const net = require("node:net");
const updater = require("./updater.cjs");
const { createDesktopBrowserHost } = require("./browser-host.cjs");
const {
  inspectSandboxProtocol,
  loadRuntimeBundle,
  runtimeManifestName,
} = require("./runtime-bundle.cjs");

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
const backendApiToken = crypto.randomBytes(32).toString("base64url");
const openTopiaProtocol = "opentopia";
const evalRuntimeFileName = "opentopia-eval-runtime.json";

/*
 * The Windows caption buttons are drawn by the OS, not by our CSS, so they have
 * to be repainted explicitly whenever the renderer resolves a different theme.
 * These values mirror --surface-chrome / --text-secondary in styles/tokens.css;
 * keep them in step with it.
 */
const titleBarOverlayColors = {
  light: { color: "#f4f7fa", symbolColor: "#5c6570" },
  dark: { color: "#1f1f1f", symbolColor: "#c2c2c2" },
};

function titleBarOverlayFor(theme) {
  const palette = titleBarOverlayColors[theme] ?? titleBarOverlayColors.light;
  return { ...palette, height: 32 };
}

let mainWindow = null;
let backendProcess = null;
let protocolClientRegistered = false;
let loggingInitialized = false;
let logFilePath = null;
let crashLogFilePath = null;
let logsDirPath = null;
let crashLogsDirPath = null;
let nextOpenRequestId = 1;
let desktopBrowserHost = null;
let desktopBrowserBroker = null;
let packagedRuntimeBundle = null;
let packagedRuntimeBundleError = null;

function evalRuntimeFilePath() {
  return (
    process.env.OPENTOPIA_EVAL_RUNTIME_FILE ||
    path.join(app.getPath("userData"), evalRuntimeFileName)
  );
}

function writeEvalRuntimeDescriptor() {
  try {
    const filePath = evalRuntimeFilePath();
    fs.mkdirSync(path.dirname(filePath), { recursive: true });
    const descriptor = {
      schemaVersion: 1,
      backendUrl: defaultBackendUrl,
      apiToken: backendApiToken,
      pid: process.pid,
      isDev,
      startedAt: new Date().toISOString(),
      launcher: process.execPath,
      launcherArgs:
        isDev && process.argv[1] ? [path.resolve(process.argv[1])] : [],
      launcherCwd: process.cwd(),
    };
    const temporaryPath = `${filePath}.tmp-${process.pid}`;
    fs.writeFileSync(temporaryPath, `${JSON.stringify(descriptor)}\n`, {
      encoding: "utf8",
      mode: 0o600,
    });
    fs.renameSync(temporaryPath, filePath);
  } catch (error) {
    logConsole("warn", "evaluation.runtime-descriptor.failed", { error });
  }
}

function removeEvalRuntimeDescriptor() {
  try {
    fs.rmSync(evalRuntimeFilePath(), { force: true });
  } catch (error) {
    logConsole("warn", "evaluation.runtime-descriptor.remove-failed", {
      error,
    });
  }
}

const secretsFilePath = "secrets.json";
const providerSecretStorageKey = "provider-api-key";
const providerSecretStoragePrefix = `${providerSecretStorageKey}:`;
const keyringProviderApiKeySourceId = `keyring:${providerSecretStorageKey}`;
const keyringProviderApiKeyEnvName = "OPENTOPIA_API_KEY";

const maxRecentWorkspaces = 12;
const maxContextSourceFiles = 20;
const maxContextSourceBytes = 25 * 1024 * 1024;
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

function isSecretName(name) {
  return /api[_-]?key|token|secret|password|authorization|credential/i.test(
    String(name || ""),
  );
}

function redactSecrets(value) {
  let output = String(value)
    .split(backendApiToken)
    .join("[redacted:api-token]");
  for (const [key, secretValue] of Object.entries(process.env)) {
    if (!isSecretName(key) || !secretValue || secretValue.length < 4) continue;
    output = output.split(secretValue).join(`[redacted:${key}]`);
  }

  return output
    .replace(/(Bearer\s+)[^\s"'`]+/gi, "$1[redacted]")
    .replace(
      /([?&][^=&\s]*(?:api[_-]?key|token|secret|password)[^=&\s]*=)[^&\s]+/gi,
      "$1[redacted]",
    )
    .replace(
      /((?:api[_-]?key|token|secret|password|authorization)\s*[:=]\s*)("[^"]*"|'[^']*'|[^\s,;]+)/gi,
      "$1[redacted]",
    )
    .replace(/\bsk-[A-Za-z0-9_-]{8,}\b/g, "[redacted-api-key]");
}

function serializeError(error) {
  if (!error) return null;
  return {
    name: error.name || "Error",
    message: redactSecrets(error.message || String(error)),
    stack: error.stack ? redactSecrets(error.stack) : undefined,
    code: error.code,
  };
}

function sanitizeForLog(value, key = "", depth = 0) {
  if (isSecretName(key)) return "[redacted]";
  if (value instanceof Error) return serializeError(value);
  if (typeof value === "string") return redactSecrets(value);
  if (
    value === null ||
    value === undefined ||
    typeof value === "number" ||
    typeof value === "boolean"
  ) {
    return value;
  }
  if (depth > 6) return "[max-depth]";
  if (Array.isArray(value)) {
    return value.map((entry) => sanitizeForLog(entry, key, depth + 1));
  }
  if (typeof value === "object") {
    const sanitized = {};
    for (const [entryKey, entryValue] of Object.entries(value)) {
      sanitized[entryKey] = sanitizeForLog(entryValue, entryKey, depth + 1);
    }
    return sanitized;
  }
  return redactSecrets(String(value));
}

function backendEndpointInfo() {
  try {
    const parsed = new URL(defaultBackendUrl);
    return {
      url: parsed.toString(),
      protocol: parsed.protocol,
      host: parsed.hostname,
      port:
        parsed.port ||
        (parsed.protocol === "https:"
          ? "443"
          : parsed.protocol === "http:"
            ? "80"
            : ""),
    };
  } catch {
    return { url: redactSecrets(defaultBackendUrl) };
  }
}

function ensureLoggingInitialized() {
  if (loggingInitialized) return;
  loggingInitialized = true;

  logsDirPath = path.join(app.getPath("userData"), "logs");
  crashLogsDirPath = path.join(logsDirPath, "crashes");
  fs.mkdirSync(crashLogsDirPath, { recursive: true });

  const timestamp = new Date().toISOString().replace(/[:.]/g, "-");
  logFilePath = path.join(
    logsDirPath,
    `startup-${timestamp}-${process.pid}.jsonl`,
  );
  crashLogFilePath = path.join(
    crashLogsDirPath,
    `crash-${timestamp}-${process.pid}.jsonl`,
  );

  writeLog("info", "app.logging.ready", {
    pid: process.pid,
    isDev,
    userData: app.getPath("userData"),
    logsDir: logsDirPath,
    crashLogsDir: crashLogsDirPath,
    backend: backendEndpointInfo(),
  });

  process.on("uncaughtExceptionMonitor", (error) => {
    writeLog("error", "process.uncaughtException", { error });
  });
  process.on("unhandledRejection", (reason) => {
    writeLog("error", "process.unhandledRejection", {
      reason: reason instanceof Error ? serializeError(reason) : reason,
    });
  });
  app.on("render-process-gone", (_event, webContents, details) => {
    writeCrashLog("error", "crash.render-process-gone", {
      url: webContents?.getURL?.(),
      details,
    });
  });
  app.on("child-process-gone", (_event, details) => {
    writeCrashLog("error", "crash.child-process-gone", { details });
  });
}

function appendLogLine(targetPath, level, event, metadata) {
  if (!targetPath) return;
  const record = {
    ts: new Date().toISOString(),
    level,
    event,
    metadata: sanitizeForLog(metadata || {}),
  };
  fs.appendFileSync(targetPath, `${JSON.stringify(record)}\n`, "utf8");
}

function writeLog(level, event, metadata = {}) {
  try {
    appendLogLine(logFilePath, level, event, metadata);
  } catch (error) {
    console.error("[opentopia] failed to write log", serializeError(error));
  }
}

function writeCrashLog(level, event, metadata = {}) {
  writeLog(level, event, metadata);
  try {
    appendLogLine(crashLogFilePath, level, event, metadata);
  } catch (error) {
    console.error(
      "[opentopia] failed to write crash log",
      serializeError(error),
    );
  }
}

function logConsole(level, message, metadata = {}) {
  writeLog(level, message, metadata);
  const line = `[opentopia] ${message}`;
  const sanitized = sanitizeForLog(metadata);
  if (level === "error") {
    console.error(line, sanitized);
  } else if (level === "warn") {
    console.warn(line, sanitized);
  } else {
    console.log(line, sanitized);
  }
}

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
  const env = {
    ...process.env,
    OPENTOPIA_DB: process.env.OPENTOPIA_DB || defaultDatabasePath,
    OPENTOPIA_PERMISSION: process.env.OPENTOPIA_PERMISSION || "auto",
    OPENTOPIA_API_TOKEN: backendApiToken,
  };

  if (isDev) {
    env.CARGO_TARGET_DIR ||=
      process.env.OPENTOPIA_DEV_CARGO_TARGET_DIR ||
      path.join(repoRoot, "target", "desktop-dev");
  }

  if (desktopBrowserBroker) {
    env.OPENTOPIA_DESKTOP_BROWSER_BROKER_URL = desktopBrowserBroker.url;
    env.OPENTOPIA_DESKTOP_BROWSER_BROKER_TOKEN = desktopBrowserBroker.token;
  }

  if (isDev) {
    env.OPENTOPIA_DEV_ORIGIN =
      process.env.VITE_DEV_SERVER_URL || "http://127.0.0.1:5173";
  }

  const selectedEnvFile = resolveOpenTopiaEnvFile(repoRoot);
  importEnvFile(env, selectedEnvFile);
  importProviderCredentialFallback(env, repoRoot, selectedEnvFile);
  applyProviderAliases(env);
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
  const extension = path.extname(filePath).toLowerCase();
  if ([".png", ".jpg", ".jpeg", ".gif", ".webp"].includes(extension)) {
    return "image";
  }
  if ([".pdf", ".docx", ".xlsx", ".pptx"].includes(extension)) {
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
  if (!mainWindow) return false;
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
      health?.apiVersion === 1;
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
  for (let i = 0; i < attempts; i += 1) {
    await new Promise((resolve) => setTimeout(resolve, 500));
    if (!backendProcess) {
      writeLog("warn", "backend.spawn.gone", { attempts: i + 1 });
      return false;
    }
    if (await isBackendHealthy()) {
      writeLog("info", "backend.spawn.ready", {
        attempts: i + 1,
        backend: backendEndpointInfo(),
      });
      return true;
    }
  }
  writeLog("error", "backend.spawn.health-timeout", {
    backend: backendEndpointInfo(),
    attempts,
  });
  return false;
}

function devServerBinaryPath(repoRoot) {
  const targetDir = cargoTargetDir(repoRoot);
  const binaryName = `opentopia-server${process.platform === "win32" ? ".exe" : ""}`;
  return path.join(targetDir, "debug", binaryName);
}

async function ensureBackendBuilt(repoRoot) {
  writeLog("info", "backend.build.starting", { repoRoot });
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
    const stderrChunks = [];
    child.stderr?.on("data", (chunk) => stderrChunks.push(chunk.toString()));
    child.on("exit", (code) => {
      if (code === 0) {
        writeLog("info", "backend.build.completed");
        resolve();
      } else {
        const stderr = stderrChunks.join("").slice(-2000);
        writeLog("error", "backend.build.failed", { code, stderr });
        reject(new Error(`cargo build exited with code ${code}`));
      }
    });
    child.on("error", (error) => {
      writeLog("error", "backend.build.error", { error: error.message });
      reject(error);
    });
  });
}

async function startBackendIfNeeded({
  waitForHealth = true,
  attempts = backendHealthAttempts,
} = {}) {
  if (await isBackendHealthy()) {
    writeEvalRuntimeDescriptor();
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
    return;
  }

  const endpoint = new URL(defaultBackendUrl);
  const endpointHost = endpoint.hostname.replace(/^\[|\]$/g, "");
  if (
    endpoint.protocol !== "http:" ||
    !["127.0.0.1", "::1", "localhost"].includes(endpointHost)
  ) {
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
    writeEvalRuntimeDescriptor();

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

async function stopManagedBackend() {
  const child = backendProcess;
  if (!child) return;

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

  mainWindow = new BrowserWindow({
    width: 1440,
    height: 900,
    minWidth: 1080,
    minHeight: 720,
    title: isDev ? "OpenTopia Dev" : "OpenTopia",
    backgroundColor: "#ffffff",
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

  desktopBrowserHost?.attachWindow(mainWindow);

  if (isDev) {
    mainWindow.on("page-title-updated", (event) => {
      event.preventDefault();
      mainWindow?.setTitle("OpenTopia Dev");
    });
  }

  mainWindow.once("ready-to-show", () => {
    mainWindow.show();
    focusMainWindow();
    flushOpenRequestsToRenderer();
  });

  mainWindow.webContents.once("did-finish-load", () => {
    writeLog("info", "window.load.finished", {
      url: mainWindow?.webContents.getURL(),
      pendingOpenRequests: openRequestHistory.length,
    });
    flushOpenRequestsToRenderer();
  });

  mainWindow.on("closed", () => {
    writeLog("info", "window.closed");
    mainWindow = null;
  });

  if (isDev) {
    mainWindow.loadURL(
      process.env.VITE_DEV_SERVER_URL || "http://127.0.0.1:5173",
    );
    mainWindow.webContents.openDevTools({ mode: "detach" });
  } else {
    mainWindow.loadFile(path.join(__dirname, "..", "dist", "index.html"));
  }

  updater.setupAutoUpdater(mainWindow);
  if (!isDev) {
    updater.checkForUpdates();
  }
}

function resolveRepoRoot() {
  return path.resolve(__dirname, "..", "..", "..");
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
      logs: logsDirPath,
      crashLogs: crashLogsDirPath,
    },
    protocol: {
      scheme: openTopiaProtocol,
      registered: protocolClientRegistered,
    },
  }));

  ipcMain.handle("platform:get-open-requests", () =>
    openRequestHistory.map((request) => ({ ...request })),
  );

  // Called by the renderer every time the resolved appearance changes.
  ipcMain.handle("platform:set-theme", (_event, theme) => {
    const resolved = theme === "dark" ? "dark" : "light";
    if (!mainWindow || mainWindow.isDestroyed()) return false;
    // Repainting the window background too avoids a pale flash on resize.
    mainWindow.setBackgroundColor(titleBarOverlayColors[resolved].color);
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
          extensions: [
            "txt",
            "md",
            "json",
            "jsonc",
            "jsonl",
            "csv",
            "tsv",
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
            "xlsx",
            "pptx",
          ],
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
      WebContentsView,
      getMainWindow: () => mainWindow,
      logger: (level, event, metadata) => logConsole(level, event, metadata),
    });
    try {
      desktopBrowserBroker = await desktopBrowserHost.startBroker();
    } catch (error) {
      logConsole("error", "browser.broker.start.failed", { error });
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

app.on("before-quit", () => {
  removeEvalRuntimeDescriptor();
  void desktopBrowserHost?.close().catch((error) => {
    logConsole("warn", "browser.host.close.failed", { error });
  });
  killBackendProcessTree();
});
