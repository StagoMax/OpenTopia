const crypto = require("node:crypto");
const fs = require("node:fs");
const http = require("node:http");
const path = require("node:path");
const { URL } = require("node:url");

const IPC_CHANNELS = Object.freeze({
  create: "browser-host:create",
  destroy: "browser-host:destroy",
  getState: "browser-host:get-state",
  navigate: "browser-host:navigate",
  navigateFromAddressBar: "browser-host:navigate-from-address-bar",
  beginUserControl: "browser-host:begin-user-control",
  back: "browser-host:back",
  forward: "browser-host:forward",
  reload: "browser-host:reload",
  setBounds: "browser-host:set-bounds",
  setVisibility: "browser-host:set-visibility",
  show: "browser-host:show",
  hide: "browser-host:hide",
  state: "browser-host:state",
});

const MAX_SESSIONS = 32;
const MAX_REQUEST_BYTES = 1024 * 1024;
const MAX_RESPONSE_BYTES = 32 * 1024 * 1024;
const MAX_URL_LENGTH = 8192;
const MAX_SELECTOR_LENGTH = 2048;
const MAX_TEXT_LENGTH = 64 * 1024;
const MAX_SNAPSHOT_BYTES = 1024 * 1024;
const MAX_SCREENSHOT_BYTES = 8 * 1024 * 1024;
const MAX_DOWNLOAD_BYTES = 100 * 1024 * 1024;
const MAX_WAIT_MS = 30_000;
const DEFAULT_WAIT_MS = 10_000;
const MAX_OPERATION_MS = 34_000;
const SCREENSHOT_CAPTURE_MS = 8_000;
const MAX_INTERACTIVE_ELEMENTS = 500;
const MAX_NETWORK_HOSTS = 256;
const MAX_OBSERVATIONS_PER_SESSION = 12;
const OBSERVATION_TTL_MS = 120_000;
const MAX_NODE_POSITION_DRIFT = 24;
const DEFAULT_PROFILE_ID = "default";
const PROFILE_ID_PATTERN = /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/;
const PROFILE_PERSISTENCE = new Set(["persistent", "ephemeral"]);
const EPHEMERAL_PARTITION_NONCE = crypto.randomBytes(16).toString("hex");
const DEFAULT_BACKGROUND_BOUNDS = Object.freeze({
  x: 0,
  y: 0,
  width: 1280,
  height: 800,
});
const SESSION_ID_PATTERN = /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/;
const ALLOWED_PROTOCOLS = new Set(["http:", "https:"]);

class BrowserHostError extends Error {
  constructor(code, message, statusCode = 400, details = null) {
    super(message);
    this.name = "BrowserHostError";
    this.code = code;
    this.statusCode = statusCode;
    this.details = details;
  }
}

function normalizeSessionId(value) {
  if (typeof value !== "string" || !SESSION_ID_PATTERN.test(value)) {
    throw new BrowserHostError(
      "invalid_session_id",
      "sessionId must be 1-128 characters using letters, numbers, '.', '_', ':' or '-'.",
    );
  }
  return value;
}

function normalizeProfileId(value) {
  const normalized = value ?? DEFAULT_PROFILE_ID;
  if (typeof normalized !== "string" || !PROFILE_ID_PATTERN.test(normalized)) {
    throw new BrowserHostError(
      "invalid_profile_id",
      "profileId must be 1-64 characters using letters, numbers, '.', '_' or '-'.",
    );
  }
  return normalized;
}

function normalizeProfilePersistence(value) {
  const normalized = value ?? "persistent";
  if (!PROFILE_PERSISTENCE.has(normalized)) {
    throw new BrowserHostError(
      "invalid_profile_persistence",
      "profilePersistence must be 'persistent' or 'ephemeral'.",
    );
  }
  return normalized;
}

function partitionForProfile(profileId, profilePersistence) {
  if (profilePersistence === "ephemeral") {
    return `opentopia-browser:${EPHEMERAL_PARTITION_NONCE}:${profileId}`;
  }
  return profileId === DEFAULT_PROFILE_ID
    ? "persist:opentopia-browser"
    : `persist:opentopia-browser:${profileId}`;
}

function normalizeUrl(value) {
  if (typeof value !== "string" || value.length === 0) {
    throw new BrowserHostError(
      "invalid_url",
      "url must be a non-empty string.",
    );
  }
  if (Buffer.byteLength(value, "utf8") > MAX_URL_LENGTH) {
    throw new BrowserHostError(
      "url_too_large",
      "url exceeds the 8 KiB limit.",
      413,
    );
  }

  let parsed;
  try {
    parsed = new URL(value);
  } catch {
    throw new BrowserHostError(
      "invalid_url",
      "url must be an absolute HTTP(S) URL.",
    );
  }
  if (!ALLOWED_PROTOCOLS.has(parsed.protocol)) {
    throw new BrowserHostError(
      "blocked_protocol",
      `Navigation to '${parsed.protocol}' URLs is blocked.`,
      403,
    );
  }
  if (parsed.username || parsed.password) {
    throw new BrowserHostError(
      "blocked_credentials",
      "URLs containing embedded credentials are blocked.",
      403,
    );
  }
  return parsed.toString();
}

function normalizeNetworkHost(value) {
  if (
    typeof value !== "string" ||
    value.trim().length === 0 ||
    /[\s/@]/.test(value)
  ) {
    throw new BrowserHostError(
      "invalid_network_host",
      "allowedHosts entries must be host names or IP addresses without ports.",
    );
  }
  const raw = value.trim().replace(/\.$/, "").toLowerCase();
  let parsed;
  try {
    parsed = new URL(`http://${raw}/`);
  } catch {
    try {
      parsed = new URL(`http://[${raw}]/`);
    } catch {
      throw new BrowserHostError(
        "invalid_network_host",
        `Invalid network host '${value}'.`,
      );
    }
  }
  if (
    parsed.username ||
    parsed.password ||
    parsed.port ||
    parsed.pathname !== "/" ||
    parsed.search ||
    parsed.hash
  ) {
    throw new BrowserHostError(
      "invalid_network_host",
      `Invalid network host '${value}'.`,
    );
  }
  return parsed.hostname.replace(/^\[|\]$/g, "").toLowerCase();
}

function normalizeAllowedHosts(value) {
  if (!Array.isArray(value) || value.length > MAX_NETWORK_HOSTS) {
    throw new BrowserHostError(
      "invalid_network_grant",
      `allowedHosts must be an array containing at most ${MAX_NETWORK_HOSTS} hosts.`,
    );
  }
  return new Set(value.map(normalizeNetworkHost));
}

function normalizeSelector(value, required = true) {
  if ((value === undefined || value === null) && !required) return null;
  if (typeof value !== "string" || value.trim().length === 0) {
    throw new BrowserHostError(
      "invalid_selector",
      "selector must be a non-empty CSS selector.",
    );
  }
  if (Buffer.byteLength(value, "utf8") > MAX_SELECTOR_LENGTH) {
    throw new BrowserHostError(
      "selector_too_large",
      "selector exceeds the 2 KiB limit.",
      413,
    );
  }
  return value;
}

function normalizeText(value, required = true) {
  if ((value === undefined || value === null) && !required) return null;
  if (typeof value !== "string") {
    throw new BrowserHostError("invalid_text", "text must be a string.");
  }
  if (Buffer.byteLength(value, "utf8") > MAX_TEXT_LENGTH) {
    throw new BrowserHostError(
      "text_too_large",
      "text exceeds the 64 KiB limit.",
      413,
    );
  }
  return value;
}

function normalizeBounds(value, window) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new BrowserHostError("invalid_bounds", "bounds must be an object.");
  }
  const fields = ["x", "y", "width", "height"];
  const parsed = {};
  for (const field of fields) {
    const number = value[field];
    if (!Number.isFinite(number) || number < 0) {
      throw new BrowserHostError(
        "invalid_bounds",
        `bounds.${field} must be a finite non-negative number.`,
      );
    }
    parsed[field] = Math.round(number);
  }

  const contentBounds = window?.getContentBounds?.();
  if (!contentBounds) return parsed;
  const maxWidth = Math.max(0, contentBounds.width - parsed.x);
  const maxHeight = Math.max(0, contentBounds.height - parsed.y);
  return {
    x: Math.min(parsed.x, contentBounds.width),
    y: Math.min(parsed.y, contentBounds.height),
    width: Math.min(parsed.width, maxWidth),
    height: Math.min(parsed.height, maxHeight),
  };
}

function truncateUtf8(value, maximumBytes) {
  const buffer = Buffer.from(String(value || ""), "utf8");
  if (buffer.length <= maximumBytes) {
    return { value: buffer.toString("utf8"), truncated: false };
  }
  let end = maximumBytes;
  while (end > 0 && (buffer[end] & 0xc0) === 0x80) end -= 1;
  return { value: buffer.subarray(0, end).toString("utf8"), truncated: true };
}

function imageLooksBlank(image) {
  if (!image || image.isEmpty()) return true;
  const size = image.getSize();
  const bitmap = image.toBitmap();
  const pixelCount = size.width * size.height;
  if (!pixelCount || bitmap.length < pixelCount * 4) return true;
  const stride = Math.max(1, Math.floor(pixelCount / 4096));
  let sampled = 0;
  let blank = 0;
  for (let pixel = 0; pixel < pixelCount; pixel += stride) {
    const offset = pixel * 4;
    const blue = bitmap[offset];
    const green = bitmap[offset + 1];
    const red = bitmap[offset + 2];
    const alpha = bitmap[offset + 3];
    sampled += 1;
    if (alpha <= 2 || (red <= 3 && green <= 3 && blue <= 3)) blank += 1;
  }
  return sampled > 0 && blank / sampled >= 0.995;
}

async function withTimeout(
  operation,
  milliseconds,
  label,
  { code = "timeout", abandonedOperation = false } = {},
) {
  let timer;
  const task = Promise.resolve().then(operation);
  const deadline = new Promise((_, reject) => {
    timer = setTimeout(() => {
      const error = new BrowserHostError(
        code,
        `${label} timed out after ${milliseconds} ms.`,
        504,
      );
      error.abandonedOperation = abandonedOperation;
      reject(error);
    }, milliseconds);
    timer.unref?.();
  });
  try {
    return await Promise.race([task, deadline]);
  } finally {
    clearTimeout(timer);
  }
}

function sleep(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function safeFilename(filename) {
  const normalized = path.basename(String(filename || "download"));
  const sanitized = normalized
    .replace(/[<>:"/\\|?*\x00-\x1f]/g, "_")
    .slice(0, 180);
  return sanitized || "download";
}

function navigationHistory(webContents) {
  return webContents.navigationHistory || webContents;
}

function canGoBack(webContents) {
  try {
    return Boolean(navigationHistory(webContents).canGoBack());
  } catch {
    return false;
  }
}

function canGoForward(webContents) {
  try {
    return Boolean(navigationHistory(webContents).canGoForward());
  } catch {
    return false;
  }
}

function browserOutput(webContents, action, contents = [], metadata = {}) {
  const currentUrl = webContents.isDestroyed() ? "" : webContents.getURL();
  return {
    url: currentUrl || null,
    contents,
    metadata: { action, ...metadata },
  };
}

function jsonContent(value) {
  return { type: "json", value };
}

function parseWaitOptions(request, defaultCondition = "document_complete") {
  if (request.wait === false) return null;
  const raw = request.wait;
  const object =
    raw && typeof raw === "object" && !Array.isArray(raw) ? raw : {};
  const numericTimeout = typeof raw === "number" ? raw : object.timeout_ms;
  const timeoutMs = Math.min(
    MAX_WAIT_MS,
    Math.max(
      0,
      Number.isFinite(numericTimeout)
        ? Math.round(numericTimeout)
        : DEFAULT_WAIT_MS,
    ),
  );
  const pollValue = object.poll_interval_ms;
  const pollIntervalMs = Math.min(
    500,
    Math.max(50, Number.isFinite(pollValue) ? Math.round(pollValue) : 100),
  );
  let condition =
    typeof object.condition === "string" ? object.condition : defaultCondition;
  if (request.selector) condition = "selector";
  else if (request.text !== undefined) condition = "text";
  if (!["document_complete", "selector", "text", "delay"].includes(condition)) {
    throw new BrowserHostError(
      "invalid_wait",
      `Unsupported wait condition '${condition}'.`,
    );
  }
  return { condition, timeoutMs, pollIntervalMs };
}

function serializeError(error) {
  if (error instanceof BrowserHostError) {
    return {
      code: error.code,
      message: error.message,
      statusCode: error.statusCode,
      details: error.details,
    };
  }
  return {
    code: "browser_host_error",
    message: error?.message || String(error),
    statusCode: 500,
  };
}

function createDesktopBrowserHost(options) {
  const {
    app,
    WebContentsView,
    nativeImage,
    getMainWindow,
    logger = () => {},
  } = options;
  const sessions = new Map();
  const sessionsByWebContentsId = new Map();
  const bearerToken = crypto.randomBytes(32).toString("base64url");
  let brokerServer = null;
  let brokerUrl = null;
  let attachedWindow = null;
  let windowSuspended = false;
  let windowListeners = [];
  const networkInterceptedSessions = new WeakSet();

  function log(level, event, metadata = {}) {
    try {
      logger(level, event, metadata);
    } catch {
      // Logging must not break browser control.
    }
  }

  function assertRenderer(event) {
    const window = getMainWindow();
    if (
      !window ||
      window.isDestroyed() ||
      event.sender !== window.webContents ||
      event.sender.isDestroyed()
    ) {
      throw new BrowserHostError(
        "forbidden_ipc_sender",
        "IPC sender is not the main renderer.",
        403,
      );
    }
    if (event.senderFrame && event.senderFrame !== event.sender.mainFrame) {
      throw new BrowserHostError(
        "forbidden_ipc_frame",
        "Browser IPC is restricted to the main frame.",
        403,
      );
    }
  }

  function sessionState(entry) {
    const webContents = entry.view.webContents;
    return {
      sessionId: entry.sessionId,
      profileId: entry.profileId,
      profilePersistence: entry.profilePersistence,
      url: webContents.isDestroyed() ? "" : webContents.getURL(),
      title: webContents.isDestroyed() ? "" : webContents.getTitle(),
      loading: webContents.isDestroyed() ? false : webContents.isLoading(),
      canGoBack: webContents.isDestroyed() ? false : canGoBack(webContents),
      canGoForward: webContents.isDestroyed()
        ? false
        : canGoForward(webContents),
      visible: Boolean(
        entry.requestedVisible &&
        !windowSuspended &&
        entry.attached &&
        entry.bounds.width > 0 &&
        entry.bounds.height > 0,
      ),
      bounds: { ...entry.bounds },
      error: entry.lastError,
    };
  }

  function browserSessionInfo(entry) {
    return {
      sessionId: entry.sessionId,
      profileId: entry.profileId,
      profilePersistence: entry.profilePersistence,
      backend: "electron",
    };
  }

  function emitState(entry) {
    if (entry.destroyed) return;
    const window = getMainWindow();
    const state = sessionState(entry);
    if (window && !window.isDestroyed() && !window.webContents.isDestroyed()) {
      window.webContents.send(IPC_CHANNELS.state, state);
    }
    return state;
  }

  function setActualVisibility(entry) {
    if (entry.destroyed) return;
    const visible = Boolean(
      entry.requestedVisible &&
      !windowSuspended &&
      entry.attached &&
      entry.bounds.width > 0 &&
      entry.bounds.height > 0,
    );
    entry.view.setVisible(visible);
  }

  function attachEntry(entry) {
    const window = getMainWindow();
    if (!window || window.isDestroyed() || entry.destroyed || entry.attached)
      return;
    window.contentView.addChildView(entry.view);
    entry.attached = true;
    entry.view.setBounds(entry.bounds);
    setActualVisibility(entry);
  }

  function detachEntry(entry) {
    const window = getMainWindow();
    if (!entry.attached) return;
    entry.view.setVisible(false);
    try {
      window?.contentView?.removeChildView(entry.view);
    } catch {
      // The owning window may already be tearing down.
    }
    entry.attached = false;
  }

  function activeTarget(entry) {
    const target = entry.targets.get(entry.activeTargetRef);
    if (!target || target.view.webContents.isDestroyed()) {
      throw new BrowserHostError(
        "target_not_found",
        "The active browser target is no longer available.",
        404,
      );
    }
    return target;
  }

  function switchActiveTarget(entry, targetRef) {
    const target = entry.targets.get(targetRef);
    if (!target || target.view.webContents.isDestroyed()) {
      throw new BrowserHostError(
        "target_not_found",
        `Browser target was not found: ${targetRef}`,
        404,
      );
    }
    if (entry.activeTargetRef === targetRef) return target;
    const wasAttached = entry.attached;
    if (wasAttached) detachEntry(entry);
    entry.activeTargetRef = targetRef;
    entry.view = target.view;
    entry.observations.clear();
    if (wasAttached) attachEntry(entry);
    emitState(entry);
    return target;
  }

  function registerTarget(entry, view, openerTargetRef = null) {
    const target = {
      targetRef: crypto.randomUUID(),
      openerTargetRef,
      view,
      frameRefs: new Map(),
    };
    entry.targets.set(target.targetRef, target);
    sessionsByWebContentsId.set(view.webContents.id, entry);
    configureRemoteContents(entry, target);
    return target;
  }

  function recordDialog(entry, target, type, message, defaultPrompt = null) {
    entry.dialogs.push({
      dialogType: String(type || "dialog"),
      message: String(message || ""),
      defaultPrompt: typeof defaultPrompt === "string" ? defaultPrompt : null,
      handled: true,
      targetRef: target.targetRef,
    });
    if (entry.dialogs.length > 32) entry.dialogs.shift();
  }

  function configureRemoteContents(entry, target) {
    const webContents = target.view.webContents;
    const browserSession = webContents.session;

    if (!networkInterceptedSessions.has(browserSession)) {
      browserSession.webRequest.onBeforeRequest(
        { urls: ["http://*/*", "https://*/*"] },
        (details, callback) => {
          const target = sessionsByWebContentsId.get(details.webContentsId);
          if (!target || !target.networkPolicyEnforced) {
            callback({});
            return;
          }
          let host = "";
          try {
            host = normalizeNetworkHost(new URL(details.url).hostname);
          } catch {
            callback({ cancel: true });
            return;
          }
          if (target.allowedHosts.has(host)) {
            callback({});
            return;
          }
          target.lastError = {
            code: "network_host_blocked",
            message: `Browser network request to '${host}' was blocked.`,
            host,
          };
          log("warn", "browser.network-request.blocked", {
            sessionId: target.sessionId,
            host,
            url: details.url,
            resourceType: details.resourceType,
          });
          callback({ cancel: true });
        },
      );
      networkInterceptedSessions.add(browserSession);
    }

    webContents.setWindowOpenHandler((details) => {
      setImmediate(async () => {
        if (entry.destroyed || !entry.targets.has(target.targetRef)) return;
        const popupView = new WebContentsView({
          webPreferences: {
            partition: entry.partition,
            nodeIntegration: false,
            contextIsolation: true,
            sandbox: true,
            webSecurity: true,
            allowRunningInsecureContent: false,
            spellcheck: false,
            backgroundThrottling: false,
          },
        });
        popupView.setBounds(DEFAULT_BACKGROUND_BOUNDS);
        const popup = registerTarget(entry, popupView, target.targetRef);
        switchActiveTarget(entry, popup.targetRef);
        log("info", "browser.target.created", {
          sessionId: entry.sessionId,
          targetRef: popup.targetRef,
          openerTargetRef: target.targetRef,
        });
        try {
          await popupView.webContents.loadURL(normalizeUrl(details.url));
        } catch (error) {
          if (entry.lastError?.code !== "network_host_blocked") {
            log("warn", "browser.target.navigation-failed", {
              sessionId: entry.sessionId,
              targetRef: popup.targetRef,
              error: serializeError(error),
            });
          }
        }
      });
      return { action: "deny" };
    });
    webContents.on("will-attach-webview", (event) => event.preventDefault());
    webContents.on("will-prevent-unload", (event) => {
      recordDialog(
        entry,
        target,
        "beforeunload",
        "The page requested confirmation before unloading.",
      );
      event.preventDefault();
    });

    try {
      const browserDebugger = webContents.debugger;
      if (!browserDebugger.isAttached()) browserDebugger.attach("1.3");
      void browserDebugger.sendCommand("Page.enable").catch((error) => {
        if (!webContents.isDestroyed()) {
          log("warn", "browser.dialog-handler.enable-failed", {
            sessionId: entry.sessionId,
            error: serializeError(error),
          });
        }
      });
      browserDebugger.on("message", (_event, method, params) => {
        if (method !== "Page.javascriptDialogOpening") return;
        recordDialog(
          entry,
          target,
          params?.type,
          params?.message,
          params?.defaultPrompt,
        );
        void browserDebugger
          .sendCommand("Page.handleJavaScriptDialog", { accept: false })
          .catch(() => {});
      });
    } catch (error) {
      log("warn", "browser.dialog-handler.unavailable", {
        sessionId: entry.sessionId,
        error: serializeError(error),
      });
    }

    const updateEvents = [
      "did-start-loading",
      "did-stop-loading",
      "did-navigate",
      "did-navigate-in-page",
      "page-title-updated",
    ];
    for (const eventName of updateEvents) {
      webContents.on(eventName, () => emitState(entry));
    }
    webContents.on("render-process-gone", (_event, details) => {
      log("error", "browser.render-process-gone", {
        sessionId: entry.sessionId,
        details,
      });
      emitState(entry);
    });
    webContents.on("destroyed", () => {
      sessionsByWebContentsId.delete(webContents.id);
      entry.targets.delete(target.targetRef);
      if (entry.destroyed || entry.activeTargetRef !== target.targetRef) return;
      const fallback =
        entry.targets.get(target.openerTargetRef) ||
        entry.targets.values().next().value;
      if (fallback) {
        try {
          getMainWindow()?.contentView?.removeChildView(target.view);
        } catch {
          // The owning window may already be tearing down.
        }
        entry.activeTargetRef = fallback.targetRef;
        entry.view = fallback.view;
        entry.attached = false;
        attachEntry(entry);
        emitState(entry);
      }
    });

    browserSession.on("will-download", (_event, item, sourceContents) => {
      if (sourceContents !== webContents || !entry.pendingDownload) return;
      const pending = entry.pendingDownload;
      entry.pendingDownload = null;
      pending.accept(item);
    });
  }

  function createSession(sessionId, options = {}) {
    const normalized = normalizeSessionId(sessionId);
    const existing = sessions.get(normalized);
    if (existing && !existing.destroyed) {
      const explicitProfile =
        options.profileId !== undefined ||
        options.profilePersistence !== undefined;
      if (explicitProfile) {
        const profileId = normalizeProfileId(options.profileId);
        const profilePersistence = normalizeProfilePersistence(
          options.profilePersistence,
        );
        if (
          existing.profileId !== profileId ||
          existing.profilePersistence !== profilePersistence
        ) {
          throw new BrowserHostError(
            "session_profile_conflict",
            `Browser session '${normalized}' is already bound to another profile.`,
            409,
          );
        }
      }
      return existing;
    }
    if (sessions.size >= MAX_SESSIONS) {
      throw new BrowserHostError(
        "too_many_sessions",
        `At most ${MAX_SESSIONS} browser sessions may be active.`,
        429,
      );
    }

    const profileId = normalizeProfileId(options.profileId);
    const profilePersistence = normalizeProfilePersistence(
      options.profilePersistence,
    );
    const partition = partitionForProfile(profileId, profilePersistence);

    const view = new WebContentsView({
      webPreferences: {
        partition,
        nodeIntegration: false,
        contextIsolation: true,
        sandbox: true,
        webSecurity: true,
        allowRunningInsecureContent: false,
        spellcheck: false,
        backgroundThrottling: false,
      },
    });
    view.setBounds(DEFAULT_BACKGROUND_BOUNDS);
    const entry = {
      sessionId: normalized,
      profileId,
      profilePersistence,
      partition,
      view,
      targets: new Map(),
      activeTargetRef: null,
      dialogs: [],
      bounds: { ...DEFAULT_BACKGROUND_BOUNDS },
      requestedVisible: false,
      attached: false,
      destroyed: false,
      pendingDownload: null,
      activeDownloadItem: null,
      lastError: null,
      observations: new Map(),
      networkPolicyEnforced: false,
      allowedHosts: new Set(),
      queue: Promise.resolve(),
    };
    sessions.set(normalized, entry);
    const target = registerTarget(entry, view);
    entry.activeTargetRef = target.targetRef;
    attachEntry(entry);
    emitState(entry);
    log("info", "browser.session.created", {
      sessionId: normalized,
      profileId,
      profilePersistence,
    });
    return entry;
  }

  function requireSession(sessionId) {
    const normalized = normalizeSessionId(sessionId);
    const entry = sessions.get(normalized);
    if (!entry || entry.destroyed) {
      throw new BrowserHostError(
        "session_not_found",
        `Browser session was not found: ${normalized}`,
        404,
      );
    }
    return entry;
  }

  async function runExclusive(entry, operation) {
    const previous = entry.queue.catch(() => {});
    let release;
    entry.queue = new Promise((resolve) => {
      release = resolve;
    });
    await previous;
    if (entry.destroyed) {
      release();
      throw new BrowserHostError(
        "session_not_found",
        "Browser session is closed.",
        404,
      );
    }
    try {
      return await withTimeout(
        operation,
        MAX_OPERATION_MS,
        "Browser operation",
        {
          code: "operation_timeout",
          abandonedOperation: true,
        },
      );
    } catch (error) {
      if (error?.abandonedOperation && !entry.destroyed) {
        log("error", "browser.session.operation-abandoned", {
          sessionId: entry.sessionId,
          error: serializeError(error),
        });
        destroySession(entry.sessionId);
      }
      throw error;
    } finally {
      release();
    }
  }

  async function navigate(entry, rawUrl, waitOptions) {
    const targetUrl = normalizeUrl(rawUrl);
    entry.lastError = null;
    const webContents = entry.view.webContents;
    try {
      await withTimeout(
        () => webContents.loadURL(targetUrl),
        MAX_WAIT_MS,
        "Navigation",
        { code: "navigation_timeout", abandonedOperation: true },
      );
    } catch (error) {
      if (entry.lastError?.code === "network_host_blocked") {
        throw new BrowserHostError(
          entry.lastError.code,
          entry.lastError.message,
          403,
          { host: entry.lastError.host },
        );
      }
      throw error;
    }
    if (waitOptions) await waitFor(entry, {}, waitOptions);
    return browserOutput(
      webContents,
      "navigate",
      [
        jsonContent({
          url: webContents.getURL(),
          title: webContents.getTitle(),
        }),
      ],
      { requested_url: targetUrl },
    );
  }

  function navigateFromAddressBar(sessionId, url) {
    const entry = requireSession(sessionId);
    return runExclusive(entry, () => {
      beginUserControl(entry);
      return navigate(entry, url, null);
    });
  }

  function beginUserControl(entry) {
    entry.networkPolicyEnforced = false;
    entry.lastError = null;
    log("info", "browser.session.control-mode-changed", {
      sessionId: entry.sessionId,
      mode: "user",
    });
    return sessionState(entry);
  }

  function frameSnapshotScript(limit) {
    return `(() => {
      const max = ${limit};
      const identities = globalThis.__opentopiaBrowserNodeIdentities ||
        (globalThis.__opentopiaBrowserNodeIdentities = { nodes: new WeakMap(), next: 0 });
      const nodeKey = (element) => {
        let key = identities.nodes.get(element);
        if (!key) {
          key = String(++identities.next);
          identities.nodes.set(element, key);
        }
        return key;
      };
      const selector = 'a[href],button,input,textarea,select,[role="button"],[role="link"],[contenteditable="true"],[tabindex]';
      const escape = (value) => window.CSS && CSS.escape ? CSS.escape(String(value)) : String(value).replace(/[^a-zA-Z0-9_-]/g, (char) => "\\\\" + char);
      const selectorFor = (element, root) => {
        if (element.id) return "#" + escape(element.id);
        const parts = [];
        for (let current = element; current && current.nodeType === Node.ELEMENT_NODE; current = current.parentElement) {
          let part = current.localName || "*";
          const siblings = current.parentElement ? Array.from(current.parentElement.children).filter((item) => item.localName === current.localName) : [];
          if (siblings.length > 1) part += ":nth-of-type(" + (siblings.indexOf(current) + 1) + ")";
          parts.unshift(part);
          if (current.parentNode === root || current === document.body) break;
        }
        return parts.join(" > ");
      };
      const roleFor = (element) => element.getAttribute("role") || ({
        a: "link", button: "button", textarea: "textbox", select: "combobox",
        input: element.type === "checkbox" ? "checkbox" : element.type === "radio" ? "radio" : "textbox"
      })[element.localName] || element.localName;
      const handoffReason = (element, name, inputType, formMethod) => {
        const normalized = [name, element.getAttribute("aria-label") || "", element.getAttribute("title") || ""].join(" ").toLowerCase();
        if (inputType === "file") return "Please choose and upload the file yourself in the visible browser, then tell me to continue.";
        if (inputType === "password" || /sign[ -]?in|log[ -]?in|password|passkey|verification|verify|captcha|one[ -]?time code|security code/.test(normalized)) return "Please complete the sign-in or verification step yourself in the visible browser, then tell me to continue.";
        if (/pay|payment|checkout|purchase|buy now|place order|subscribe/.test(normalized)) return "Please review and complete the payment or purchase yourself in the visible browser, then tell me to continue.";
        if (/send|publish|post|share|upload|delete|remove|submit|save changes|confirm/.test(normalized) && formMethod !== "get") return "Please review and complete this external action yourself in the visible browser, then tell me to continue.";
        return null;
      };
      const nodes = [];
      const walk = (root, shadowPath) => {
        for (const element of root.querySelectorAll(selector)) {
          if (nodes.length >= max) break;
          if (element.disabled || !element.getClientRects().length) continue;
          const rect = element.getBoundingClientRect();
          const name = String(element.innerText || element.value || element.getAttribute("aria-label") || element.getAttribute("placeholder") || "").slice(0, 2048);
          const inputType = (element.getAttribute("type") || "").toLowerCase() || null;
          const formMethod = (element.getAttribute("formmethod") || element.form?.getAttribute("method") || "get").toLowerCase();
          const userActionReason = handoffReason(element, name, inputType, formMethod);
          nodes.push({
            selectorPath: [...shadowPath, selectorFor(element, root)],
            nodeKey: nodeKey(element),
            tagName: element.localName,
            role: roleFor(element),
            name,
            href: element.href || null,
            formAction: element.formAction || element.form?.action || null,
            formMethod,
            inputType,
            editable: Boolean(element.isContentEditable || (["input", "textarea", "select"].includes(element.localName) && !element.readOnly)),
            requiresUserAction: Boolean(userActionReason),
            userActionReason,
            bounds: { x: rect.x, y: rect.y, width: rect.width, height: rect.height },
          });
        }
        for (const host of root.querySelectorAll("*")) {
          if (nodes.length >= max) break;
          if (host.shadowRoot) walk(host.shadowRoot, [...shadowPath, selectorFor(host, root)]);
        }
      };
      walk(document, []);
      return { url: document.location.href, name: window.name || "", text: document.body ? document.body.innerText : "", nodes };
    })()`;
  }

  async function collectStructuredSnapshot(entry) {
    const target = activeTarget(entry);
    const webContents = target.view.webContents;
    const frameList = webContents.mainFrame?.framesInSubtree || [
      webContents.mainFrame,
    ];
    const frames = [];
    const interactiveElements = [];
    const texts = [];
    for (const frame of frameList) {
      if (!frame || interactiveElements.length >= MAX_INTERACTIVE_ELEMENTS)
        break;
      const key = `${frame.processId}:${frame.routingId}`;
      if (!target.frameRefs.has(key))
        target.frameRefs.set(key, crypto.randomUUID());
      const frameRef = target.frameRefs.get(key);
      const parentKey = frame.parent
        ? `${frame.parent.processId}:${frame.parent.routingId}`
        : null;
      if (parentKey && !target.frameRefs.has(parentKey))
        target.frameRefs.set(parentKey, crypto.randomUUID());
      try {
        const result = await frame.executeJavaScript(
          frameSnapshotScript(
            MAX_INTERACTIVE_ELEMENTS - interactiveElements.length,
          ),
          false,
        );
        frames.push({
          frameRef,
          targetRef: target.targetRef,
          parentFrameRef: parentKey ? target.frameRefs.get(parentKey) : null,
          url: String(result?.url || frame.url || ""),
          name: String(result?.name || ""),
        });
        if (result?.text) texts.push(String(result.text));
        for (const node of result?.nodes || []) {
          interactiveElements.push({
            ...node,
            targetRef: target.targetRef,
            frameRef,
            locator: {
              targetRef: target.targetRef,
              frameRef,
              processId: frame.processId,
              routingId: frame.routingId,
              selectorPath: node.selectorPath,
              nodeKey: node.nodeKey,
            },
          });
        }
      } catch (error) {
        if (frame === webContents.mainFrame) throw error;
      }
    }
    const text = truncateUtf8(texts.join("\n"), MAX_SNAPSHOT_BYTES);
    let accessibilityTree = [];
    try {
      const ax = await withDebugger(webContents, (browserDebugger) =>
        browserDebugger.sendCommand("Accessibility.getFullAXTree", {
          depth: 32,
        }),
      );
      const rootFrameRef =
        frames.find((frame) => !frame.parentFrameRef)?.frameRef || null;
      accessibilityTree = (ax?.nodes || []).slice(0, 1000).map((node) => {
        const axValue = (field) => {
          const value = field?.value;
          return value == null ? "" : String(value);
        };
        return {
          axNodeId: String(node.nodeId || ""),
          parentAxNodeId: node.parentId ? String(node.parentId) : null,
          role: axValue(node.role),
          name: axValue(node.name),
          value: axValue(node.value) || null,
          description: axValue(node.description) || null,
          ignored: Boolean(node.ignored),
          targetRef: target.targetRef,
          frameRef: rootFrameRef,
          nodeRef: null,
        };
      });
    } catch (error) {
      log("warn", "browser.accessibility-tree.failed", {
        sessionId: entry.sessionId,
        error: serializeError(error),
      });
    }
    const targets = [...entry.targets.values()].map((item) => ({
      targetRef: item.targetRef,
      url: item.view.webContents.isDestroyed()
        ? ""
        : item.view.webContents.getURL(),
      title: item.view.webContents.isDestroyed()
        ? ""
        : item.view.webContents.getTitle(),
      active: item.targetRef === entry.activeTargetRef,
      opener: item.openerTargetRef,
    }));
    return {
      url: webContents.getURL(),
      title: webContents.getTitle(),
      text: text.value,
      textTruncated: text.truncated,
      interactiveElements,
      targets,
      frames,
      accessibilityTree,
    };
  }

  async function snapshot(entry, includeInternalLocators = false) {
    const webContents = entry.view.webContents;
    const value = await collectStructuredSnapshot(entry);
    const outputValue = includeInternalLocators
      ? value
      : {
          ...value,
          interactiveElements: value.interactiveElements.map(
            ({
              locator: _locator,
              selectorPath: _selectorPath,
              nodeKey: _nodeKey,
              ...node
            }) => node,
          ),
        };
    return browserOutput(
      webContents,
      "snapshot",
      [
        { type: "text", text: value.text, truncated: value.textTruncated },
        jsonContent(outputValue),
      ],
      {
        title: value.title,
        interactive_elements_truncated:
          value.interactiveElements.length >= MAX_INTERACTIVE_ELEMENTS,
      },
    );
  }

  function staleObservation(reason) {
    return new BrowserHostError("stale_observation", reason, 409);
  }

  function pruneObservations(entry) {
    const now = Date.now();
    for (const [id, observation] of entry.observations) {
      if (now - observation.capturedAt > OBSERVATION_TTL_MS) {
        entry.observations.delete(id);
      }
    }
    while (entry.observations.size > MAX_OBSERVATIONS_PER_SESSION) {
      entry.observations.delete(entry.observations.keys().next().value);
    }
  }

  function snapshotValue(output) {
    return output.contents.find((content) => content?.type === "json")?.value;
  }

  async function observe(entry, includeScreenshot) {
    const output = await snapshot(entry, true);
    const snapshotValueResult = snapshotValue(output);
    if (!snapshotValueResult || typeof snapshotValueResult !== "object") {
      throw new BrowserHostError(
        "observation_failed",
        "Browser snapshot did not include structured page data.",
        500,
      );
    }
    const observationId = crypto.randomUUID();
    const nodes = [];
    const bindings = new Map();
    for (const raw of snapshotValueResult.interactiveElements || []) {
      if (!raw || !raw.locator || !Array.isArray(raw.locator.selectorPath))
        continue;
      const nodeRef = crypto.randomUUID();
      const node = {
        nodeRef,
        role: String(raw.role || raw.tagName || "element"),
        name: String(raw.name || ""),
        tagName: String(raw.tagName || ""),
        bounds: raw.bounds || { x: 0, y: 0, width: 0, height: 0 },
        targetRef: typeof raw.targetRef === "string" ? raw.targetRef : null,
        frameRef: typeof raw.frameRef === "string" ? raw.frameRef : null,
        href: typeof raw.href === "string" ? raw.href : null,
        formAction: typeof raw.formAction === "string" ? raw.formAction : null,
        formMethod: typeof raw.formMethod === "string" ? raw.formMethod : null,
        inputType: typeof raw.inputType === "string" ? raw.inputType : null,
        editable: Boolean(raw.editable),
        requiresUserAction: Boolean(raw.requiresUserAction),
        userActionReason:
          typeof raw.userActionReason === "string"
            ? raw.userActionReason
            : null,
      };
      nodes.push(node);
      bindings.set(nodeRef, { node, locator: raw.locator });
    }
    entry.observations.set(observationId, {
      capturedAt: Date.now(),
      url: String(snapshotValueResult.url || entry.view.webContents.getURL()),
      targetRef: entry.activeTargetRef,
      nodes: bindings,
    });
    pruneObservations(entry);
    let screenshotValue = null;
    if (includeScreenshot) {
      const screenshotOutput = await screenshot(entry);
      const image = screenshotOutput.contents.find(
        (content) => content?.type === "image",
      );
      if (image) {
        screenshotValue = {
          mimeType: image.mime_type,
          bytes: image.bytes,
        };
      }
    }
    return {
      observationId,
      url: String(snapshotValueResult.url || entry.view.webContents.getURL()),
      title: String(
        snapshotValueResult.title || entry.view.webContents.getTitle(),
      ),
      text: String(snapshotValueResult.text || ""),
      textTruncated: Boolean(snapshotValueResult.textTruncated),
      nodes,
      targets: snapshotValueResult.targets || [],
      frames: snapshotValueResult.frames || [],
      accessibilityTree: snapshotValueResult.accessibilityTree || [],
      dialogs: entry.dialogs.splice(0),
      screenshot: screenshotValue,
    };
  }

  function observedNode(entry, rawObservationId, rawNodeRef) {
    if (
      typeof rawObservationId !== "string" ||
      typeof rawNodeRef !== "string"
    ) {
      throw new BrowserHostError(
        "invalid_observation",
        "observationId and nodeRef are required.",
      );
    }
    pruneObservations(entry);
    const observation = entry.observations.get(rawObservationId);
    if (!observation) {
      throw staleObservation("The observation is missing or expired.");
    }
    const node = observation.nodes.get(rawNodeRef);
    if (!node) {
      throw staleObservation("The node does not belong to this observation.");
    }
    return { observation, binding: node };
  }

  function nodesMatch(expected, current) {
    const bounds = expected.bounds || {};
    const currentBounds = current.bounds || {};
    return (
      expected.role === current.role &&
      expected.name === current.name &&
      expected.tagName === current.tagName &&
      expected.targetRef === current.targetRef &&
      expected.frameRef === current.frameRef &&
      expected.href === current.href &&
      expected.formAction === current.formAction &&
      expected.formMethod === current.formMethod &&
      expected.inputType === current.inputType &&
      expected.editable === current.editable &&
      expected.requiresUserAction === current.requiresUserAction &&
      expected.userActionReason === current.userActionReason &&
      Math.abs(Number(bounds.x) - Number(currentBounds.x)) <=
        MAX_NODE_POSITION_DRIFT &&
      Math.abs(Number(bounds.y) - Number(currentBounds.y)) <=
        MAX_NODE_POSITION_DRIFT &&
      Math.abs(Number(bounds.width) - Number(currentBounds.width)) <=
        MAX_NODE_POSITION_DRIFT &&
      Math.abs(Number(bounds.height) - Number(currentBounds.height)) <=
        MAX_NODE_POSITION_DRIFT
    );
  }

  function locatorsMatch(left, right) {
    return Boolean(
      left &&
      right &&
      left.targetRef === right.targetRef &&
      left.frameRef === right.frameRef &&
      left.processId === right.processId &&
      left.routingId === right.routingId &&
      left.nodeKey === right.nodeKey &&
      Array.isArray(left.selectorPath) &&
      Array.isArray(right.selectorPath) &&
      left.selectorPath.length === right.selectorPath.length &&
      left.selectorPath.every(
        (part, index) => part === right.selectorPath[index],
      ),
    );
  }

  function frameForLocator(entry, locator) {
    const target = activeTarget(entry);
    if (target.targetRef !== locator?.targetRef) {
      throw staleObservation(
        "The active browser target changed after the observation.",
      );
    }
    const frames = target.view.webContents.mainFrame?.framesInSubtree || [
      target.view.webContents.mainFrame,
    ];
    const frame = frames.find(
      (candidate) =>
        candidate.processId === locator.processId &&
        candidate.routingId === locator.routingId,
    );
    if (!frame)
      throw staleObservation(
        "The observed frame navigated or no longer exists.",
      );
    return frame;
  }

  async function performLocator(entry, locator, request) {
    const frame = frameForLocator(entry, locator);
    const result = await frame.executeJavaScript(
      `(() => {
        const path = ${JSON.stringify(locator.selectorPath)};
        let root = document;
        let element = null;
        for (let index = 0; index < path.length; index += 1) {
          element = root.querySelector(path[index]);
          if (!element) return { found: false };
          if (index + 1 < path.length) {
            root = element.shadowRoot;
            if (!root) return { found: false };
          }
        }
        element.scrollIntoView({ block: "center", inline: "center" });
        const operation = ${JSON.stringify(request.operation)};
        const value = ${JSON.stringify(typeof request.value === "string" ? request.value : typeof request.text === "string" ? request.text : "")};
        if (operation === "click") {
          element.click();
        } else if (operation === "type") {
          element.focus();
          const next = ${request.clearFirst !== false} ? value : String(element.value ?? element.textContent ?? "") + value;
          if (element.isContentEditable) element.textContent = next;
          else if ("value" in element) {
            const descriptor = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(element), "value");
            if (descriptor?.set) descriptor.set.call(element, next); else element.value = next;
          } else return { found: true, supported: false };
          element.dispatchEvent(new InputEvent("input", { bubbles: true, composed: true, inputType: "insertText", data: value }));
          element.dispatchEvent(new Event("change", { bubbles: true, composed: true }));
        } else if (operation === "select") {
          if (!(element instanceof HTMLSelectElement)) return { found: true, supported: false };
          const option = Array.from(element.options).find((candidate) => candidate.value === value || candidate.label === value);
          if (!option) return { found: true, optionFound: false };
          element.value = option.value;
          element.dispatchEvent(new Event("input", { bubbles: true, composed: true }));
          element.dispatchEvent(new Event("change", { bubbles: true, composed: true }));
        } else if (operation === "hover") {
          const rect = element.getBoundingClientRect();
          const init = { bubbles: true, composed: true, clientX: rect.x + rect.width / 2, clientY: rect.y + rect.height / 2 };
          if (typeof PointerEvent === "function") element.dispatchEvent(new PointerEvent("pointerover", init));
          element.dispatchEvent(new MouseEvent("mouseover", init));
          element.dispatchEvent(new MouseEvent("mouseenter", { ...init, bubbles: false }));
          element.dispatchEvent(new MouseEvent("mousemove", init));
        } else if (operation === "scroll") {
          const scroller = element.scrollHeight > element.clientHeight || element.scrollWidth > element.clientWidth ? element : window;
          scroller.scrollBy({ left: ${Number(request.deltaX) || 0}, top: ${Number(request.deltaY) || 0}, behavior: "instant" });
        } else return { found: true, supported: false };
        return { found: true, supported: true, optionFound: true };
      })()`,
      true,
    );
    if (!result?.found)
      throw staleObservation("The observed element no longer exists.");
    if (result.supported === false) {
      throw new BrowserHostError(
        "unsupported_element_action",
        `The observed element does not support ${request.operation}.`,
        409,
      );
    }
    if (result.optionFound === false) {
      throw new BrowserHostError(
        "select_option_not_found",
        "The requested select option does not exist.",
        409,
      );
    }
  }

  async function perform(entry, request) {
    const { observation, binding } = observedNode(
      entry,
      request.observationId,
      request.nodeRef,
    );
    const webContents = entry.view.webContents;
    if (entry.activeTargetRef !== observation.targetRef) {
      throw staleObservation(
        "The active browser target changed after the observation.",
      );
    }
    if (webContents.getURL() !== observation.url) {
      throw staleObservation("The page URL changed after the observation.");
    }
    const output = await snapshot(entry, true);
    const before = snapshotValue(output);
    const current = before?.interactiveElements?.find((node) =>
      locatorsMatch(node?.locator, binding.locator),
    );
    if (!current) {
      throw staleObservation("The observed element no longer exists.");
    }
    const currentNode = {
      ...current,
      nodeRef: binding.node.nodeRef,
      targetRef:
        typeof current.targetRef === "string" ? current.targetRef : null,
      frameRef: typeof current.frameRef === "string" ? current.frameRef : null,
      href: typeof current.href === "string" ? current.href : null,
      formAction:
        typeof current.formAction === "string" ? current.formAction : null,
      formMethod:
        typeof current.formMethod === "string" ? current.formMethod : null,
      inputType:
        typeof current.inputType === "string" ? current.inputType : null,
      editable: Boolean(current.editable),
      requiresUserAction: Boolean(current.requiresUserAction),
      userActionReason:
        typeof current.userActionReason === "string"
          ? current.userActionReason
          : null,
    };
    if (!nodesMatch(binding.node, currentNode)) {
      throw staleObservation("The observed element changed or moved.");
    }
    if (request.operation === "type" && !currentNode.editable) {
      throw staleObservation("The observed element is no longer editable.");
    }
    if (request.operation === "click")
      validateElementNavigation(currentNode, webContents.getURL());
    if (
      !new Set(["click", "type", "select", "hover", "scroll"]).has(
        request.operation,
      )
    ) {
      throw new BrowserHostError(
        "invalid_action",
        "operation must be click, type, select, hover, or scroll.",
      );
    }
    await performLocator(entry, binding.locator, request);
    await sleep(50);
    const after = snapshotValue(await snapshot(entry, true));
    const urlChanged = String(before?.url || "") !== String(after?.url || "");
    const titleChanged =
      String(before?.title || "") !== String(after?.title || "");
    const textChanged =
      String(before?.text || "") !== String(after?.text || "");
    return {
      observationId: request.observationId,
      nodeRef: request.nodeRef,
      action: request.operation,
      target: currentNode,
      url: entry.view.webContents.getURL(),
      title: entry.view.webContents.getTitle(),
      verification: {
        pageChanged: urlChanged || titleChanged || textChanged,
        urlChanged,
        titleChanged,
        textChanged,
      },
    };
  }

  async function withDebugger(webContents, operation, timeoutOptions = null) {
    const browserDebugger = webContents.debugger;
    let attachedHere = false;
    if (!browserDebugger.isAttached()) {
      browserDebugger.attach("1.3");
      attachedHere = true;
    }
    try {
      if (!timeoutOptions) return await operation(browserDebugger);
      return await withTimeout(
        () => operation(browserDebugger),
        timeoutOptions.milliseconds,
        timeoutOptions.label,
        { code: timeoutOptions.code },
      );
    } finally {
      if (attachedHere && browserDebugger.isAttached())
        browserDebugger.detach();
    }
  }

  async function screenshot(entry) {
    const webContents = entry.view.webContents;
    const surfaceWasVisible = sessionState(entry).visible;
    let bytes;
    let captureBackend = "capture_page";
    let fallbackReason = null;
    try {
      if (!surfaceWasVisible) {
        entry.view.setBounds({
          ...entry.bounds,
          x: -entry.bounds.width - 1,
        });
        entry.view.setVisible(true);
        await sleep(50);
      }
      const image = await withTimeout(
        () =>
          webContents.capturePage(undefined, {
            stayHidden: false,
            stayAwake: true,
          }),
        SCREENSHOT_CAPTURE_MS,
        "Electron screenshot capture",
        { code: "screenshot_capture_timeout" },
      );
      if (imageLooksBlank(image)) {
        throw new BrowserHostError(
          "screenshot_capture_blank",
          "Electron screenshot capture returned an empty or blank image.",
          500,
        );
      }
      bytes = image.toPNG();
    } catch (primaryError) {
      captureBackend = "devtools";
      fallbackReason = serializeError(primaryError).code;
      log("warn", "browser.screenshot.capture-page-failed", {
        sessionId: entry.sessionId,
        error: serializeError(primaryError),
      });
      const result = await withDebugger(
        webContents,
        (browserDebugger) =>
          browserDebugger.sendCommand("Page.captureScreenshot", {
            format: "png",
            fromSurface: true,
            captureBeyondViewport: false,
          }),
        {
          milliseconds: SCREENSHOT_CAPTURE_MS,
          label: "DevTools screenshot capture",
          code: "screenshot_fallback_timeout",
        },
      );
      if (!result?.data) {
        throw new BrowserHostError(
          "screenshot_failed",
          "Screenshot capture returned no image data.",
          500,
        );
      }
      bytes = Buffer.from(result.data, "base64");
      if (nativeImage && imageLooksBlank(nativeImage.createFromBuffer(bytes))) {
        throw new BrowserHostError(
          "screenshot_blank",
          "Both screenshot backends returned an empty or blank image.",
          500,
        );
      }
    } finally {
      if (!surfaceWasVisible && !entry.destroyed) {
        entry.view.setVisible(false);
        entry.view.setBounds(entry.bounds);
        setActualVisibility(entry);
      }
    }
    if (bytes.length > MAX_SCREENSHOT_BYTES) {
      throw new BrowserHostError(
        "screenshot_too_large",
        `Screenshot is ${bytes.length} bytes; maximum is ${MAX_SCREENSHOT_BYTES}.`,
        413,
      );
    }
    return browserOutput(
      webContents,
      "screenshot",
      [
        {
          type: "image",
          mime_type: "image/png",
          bytes: bytes.toString("base64"),
        },
      ],
      {
        captureBackend,
        fallbackReason,
      },
    );
  }

  async function locateElement(entry, rawSelector) {
    const selector = normalizeSelector(rawSelector);
    const serialized = JSON.stringify(selector);
    const result = await entry.view.webContents.executeJavaScript(
      `(() => {
        const element = document.querySelector(${serialized});
        if (!element) return null;
        element.scrollIntoView({ block: "center", inline: "center" });
        const rect = element.getBoundingClientRect();
        if (!rect.width || !rect.height) return { hidden: true };
        return {
          x: rect.left + rect.width / 2,
          y: rect.top + rect.height / 2,
          href: element.getAttribute("href"),
          formAction: element.getAttribute("formaction") || (element.form && element.form.getAttribute("action"))
        };
      })()`,
      false,
    );
    if (!result) {
      throw new BrowserHostError(
        "selector_not_found",
        `No element matched selector: ${selector}`,
        404,
      );
    }
    if (result.hidden) {
      throw new BrowserHostError(
        "element_not_visible",
        `Element is not visible: ${selector}`,
      );
    }
    return {
      selector,
      x: Math.round(result.x),
      y: Math.round(result.y),
      href: result.href,
      formAction: result.formAction,
    };
  }

  function validateElementNavigation(element, currentUrl) {
    for (const candidate of [element.href, element.formAction]) {
      if (!candidate) continue;
      let resolved;
      try {
        resolved = new URL(candidate, currentUrl).toString();
      } catch {
        throw new BrowserHostError(
          "invalid_element_navigation",
          "The selected element contains an invalid navigation target.",
        );
      }
      normalizeUrl(resolved);
    }
  }

  async function click(entry, rawSelector) {
    const element = await locateElement(entry, rawSelector);
    const webContents = entry.view.webContents;
    validateElementNavigation(element, webContents.getURL());
    if (entry.requestedVisible && !windowSuspended) {
      await withDebugger(webContents, async (browserDebugger) => {
        await browserDebugger.sendCommand("Input.dispatchMouseEvent", {
          type: "mousePressed",
          x: element.x,
          y: element.y,
          button: "left",
          clickCount: 1,
        });
        await browserDebugger.sendCommand("Input.dispatchMouseEvent", {
          type: "mouseReleased",
          x: element.x,
          y: element.y,
          button: "left",
          clickCount: 1,
        });
      });
    } else {
      const serializedSelector = JSON.stringify(element.selector);
      await webContents.executeJavaScript(
        `(() => {
          const target = document.querySelector(${serializedSelector});
          if (!target) throw new Error("Element no longer exists");
          target.click();
        })()`,
        false,
      );
    }
    await sleep(50);
    return browserOutput(
      webContents,
      "click",
      [
        jsonContent({
          url: webContents.getURL(),
          title: webContents.getTitle(),
        }),
      ],
      { selector: element.selector },
    );
  }

  async function typeText(entry, rawSelector, rawText) {
    const selector = normalizeSelector(rawSelector);
    const text = normalizeText(rawText);
    const serializedSelector = JSON.stringify(selector);
    const serializedText = JSON.stringify(text);
    const result = await entry.view.webContents.executeJavaScript(
      `(() => {
        const element = document.querySelector(${serializedSelector});
        if (!element) return { found: false };
        element.scrollIntoView({ block: "center", inline: "center" });
        element.focus();
        const value = ${serializedText};
        if (element.isContentEditable) {
          element.textContent = value;
        } else if ("value" in element) {
          const prototype = Object.getPrototypeOf(element);
          const descriptor = prototype && Object.getOwnPropertyDescriptor(prototype, "value");
          if (descriptor && descriptor.set) descriptor.set.call(element, value);
          else element.value = value;
        } else {
          return { found: true, editable: false };
        }
        element.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: value }));
        element.dispatchEvent(new Event("change", { bubbles: true }));
        return { found: true, editable: true };
      })()`,
      false,
    );
    if (!result?.found) {
      throw new BrowserHostError(
        "selector_not_found",
        `No element matched selector: ${selector}`,
        404,
      );
    }
    if (!result.editable) {
      throw new BrowserHostError(
        "element_not_editable",
        `Element is not editable: ${selector}`,
      );
    }
    return browserOutput(
      entry.view.webContents,
      "type",
      [
        jsonContent({
          url: entry.view.webContents.getURL(),
          title: entry.view.webContents.getTitle(),
        }),
      ],
      { selector, text_bytes: Buffer.byteLength(text, "utf8") },
    );
  }

  async function waitFor(entry, request, parsedOptions = null) {
    const options = parsedOptions || parseWaitOptions(request);
    if (!options)
      return browserOutput(entry.view.webContents, "wait", [], {
        skipped: true,
      });
    const selector =
      options.condition === "selector"
        ? normalizeSelector(request.selector)
        : null;
    const text =
      options.condition === "text" ? normalizeText(request.text) : null;
    if (options.condition === "delay") {
      await sleep(options.timeoutMs);
    } else {
      const startedAt = Date.now();
      let matched = false;
      while (Date.now() - startedAt <= options.timeoutMs) {
        matched = await entry.view.webContents.executeJavaScript(
          `(() => {
            const condition = ${JSON.stringify(options.condition)};
            if (condition === "document_complete") return document.readyState !== "loading";
            if (condition === "selector") return Boolean(document.querySelector(${JSON.stringify(selector)}));
            return Boolean(document.body && document.body.innerText.includes(${JSON.stringify(text)}));
          })()`,
          false,
        );
        if (matched) break;
        await sleep(options.pollIntervalMs);
      }
      if (!matched) {
        throw new BrowserHostError(
          "timeout",
          `Wait condition '${options.condition}' timed out after ${options.timeoutMs} ms.`,
          504,
        );
      }
    }
    return browserOutput(
      entry.view.webContents,
      "wait",
      [
        jsonContent({
          url: entry.view.webContents.getURL(),
          title: entry.view.webContents.getTitle(),
        }),
      ],
      { condition: options.condition },
    );
  }

  async function download(entry, rawUrl) {
    const targetUrl = normalizeUrl(rawUrl);
    if (entry.pendingDownload) {
      throw new BrowserHostError(
        "download_in_progress",
        "A download is already in progress.",
        409,
      );
    }
    const downloadDirectory = path.join(app.getPath("downloads"), "OpenTopia");
    fs.mkdirSync(downloadDirectory, { recursive: true });

    let downloadTimeout;
    const resultPromise = new Promise((resolve, reject) => {
      downloadTimeout = setTimeout(() => {
        entry.pendingDownload = null;
        entry.activeDownloadItem?.cancel();
        entry.activeDownloadItem = null;
        reject(new BrowserHostError("timeout", "Download timed out.", 504));
      }, MAX_WAIT_MS);
      downloadTimeout.unref?.();

      entry.pendingDownload = {
        accept(item) {
          entry.activeDownloadItem = item;
          const filename = `${Date.now()}-${crypto.randomBytes(4).toString("hex")}-${safeFilename(item.getFilename())}`;
          const savePath = path.join(downloadDirectory, filename);
          item.setSavePath(savePath);
          item.on("updated", () => {
            if (item.getReceivedBytes() > MAX_DOWNLOAD_BYTES) item.cancel();
          });
          item.once("done", (_event, state) => {
            clearTimeout(downloadTimeout);
            entry.activeDownloadItem = null;
            if (state !== "completed") {
              reject(
                new BrowserHostError(
                  state === "cancelled"
                    ? "download_too_large_or_cancelled"
                    : "download_failed",
                  `Download ended with state '${state}'.`,
                  state === "cancelled" ? 413 : 500,
                ),
              );
              return;
            }
            const stat = fs.statSync(savePath);
            if (stat.size > MAX_DOWNLOAD_BYTES) {
              fs.rmSync(savePath, { force: true });
              reject(
                new BrowserHostError(
                  "download_too_large",
                  `Download exceeds the ${MAX_DOWNLOAD_BYTES} byte limit.`,
                  413,
                ),
              );
              return;
            }
            resolve({
              path: savePath,
              filename: path.basename(savePath),
              bytes: stat.size,
              mime_type: item.getMimeType() || null,
            });
          });
        },
      };
    });

    try {
      entry.view.webContents.downloadURL(targetUrl);
    } catch (error) {
      clearTimeout(downloadTimeout);
      entry.pendingDownload = null;
      throw error;
    }
    const downloadResult = await resultPromise;
    return browserOutput(
      entry.view.webContents,
      "download",
      [
        {
          type: "file",
          path: downloadResult.path,
          mime_type: downloadResult.mime_type,
          bytes: downloadResult.bytes,
        },
      ],
      { filename: downloadResult.filename, requested_url: targetUrl },
    );
  }

  function setBounds(entry, rawBounds) {
    const window = getMainWindow();
    const bounds = normalizeBounds(rawBounds, window);
    entry.bounds = bounds;
    entry.view.setBounds(bounds);
    setActualVisibility(entry);
    return emitState(entry);
  }

  function setVisibility(entry, visible) {
    if (typeof visible !== "boolean") {
      throw new BrowserHostError(
        "invalid_visibility",
        "visible must be a boolean.",
      );
    }
    entry.requestedVisible = visible;
    if (visible) attachEntry(entry);
    setActualVisibility(entry);
    return emitState(entry);
  }

  function destroySession(sessionId) {
    const entry = requireSession(sessionId);
    entry.requestedVisible = false;
    entry.pendingDownload = null;
    entry.activeDownloadItem?.cancel();
    entry.activeDownloadItem = null;
    detachEntry(entry);
    entry.destroyed = true;
    for (const target of [...entry.targets.values()]) {
      sessionsByWebContentsId.delete(target.view.webContents.id);
      if (!target.view.webContents.isDestroyed())
        target.view.webContents.close();
    }
    entry.targets.clear();
    sessions.delete(entry.sessionId);
    log("info", "browser.session.destroyed", { sessionId: entry.sessionId });
    return { sessionId: entry.sessionId, destroyed: true };
  }

  function destroyAllSessions() {
    for (const sessionId of [...sessions.keys()]) {
      try {
        destroySession(sessionId);
      } catch {
        // Continue tearing down the remaining views.
      }
    }
  }

  async function executeAction(request) {
    if (!request || typeof request !== "object" || Array.isArray(request)) {
      throw new BrowserHostError(
        "invalid_request",
        "Request body must be a JSON object.",
      );
    }
    const sessionId = normalizeSessionId(request.sessionId);
    const action = request.action;
    if (typeof action !== "string") {
      throw new BrowserHostError("invalid_action", "action must be a string.");
    }
    const supported = new Set([
      "create_session",
      "navigate",
      "snapshot",
      "observe",
      "observation_node",
      "switch_target",
      "screenshot",
      "perform",
      "wait",
      "download",
      "grant_network_access",
      "close",
    ]);
    if (!supported.has(action)) {
      throw new BrowserHostError(
        "invalid_action",
        `Unsupported browser action '${action}'.`,
      );
    }

    if (action === "create_session") {
      const entry = createSession(sessionId, {
        profileId: request.profileId,
        profilePersistence: request.profilePersistence,
      });
      return browserSessionInfo(entry);
    }

    if (action === "close") {
      const entry = requireSession(sessionId);
      return runExclusive(entry, async () => {
        const output = browserOutput(entry.view.webContents, "close", [], {
          closed: true,
        });
        destroySession(sessionId);
        return output;
      });
    }

    const entry = createSession(sessionId);
    return runExclusive(entry, async () => {
      switch (action) {
        case "grant_network_access": {
          const allowedHosts = normalizeAllowedHosts(request.allowedHosts);
          if (
            new Set([...entry.allowedHosts, ...allowedHosts]).size >
            MAX_NETWORK_HOSTS
          ) {
            throw new BrowserHostError(
              "invalid_network_grant",
              `A browser session may authorize at most ${MAX_NETWORK_HOSTS} hosts.`,
            );
          }
          entry.networkPolicyEnforced = true;
          for (const host of allowedHosts) entry.allowedHosts.add(host);
          log("info", "browser.session.control-mode-changed", {
            sessionId: entry.sessionId,
            mode: "automation",
          });
          return browserOutput(entry.view.webContents, action, [], {
            allowedHosts: [...entry.allowedHosts].sort(),
          });
        }
        case "navigate":
          return navigate(entry, request.url, parseWaitOptions(request));
        case "snapshot":
          return snapshot(entry);
        case "observe":
          return observe(entry, Boolean(request.includeScreenshot));
        case "observation_node":
          return observedNode(entry, request.observationId, request.nodeRef)
            .binding.node;
        case "switch_target": {
          if (typeof request.targetRef !== "string" || !request.targetRef) {
            throw new BrowserHostError(
              "invalid_target",
              "targetRef is required.",
            );
          }
          const target = switchActiveTarget(entry, request.targetRef);
          return browserOutput(
            target.view.webContents,
            action,
            [
              jsonContent({
                url: target.view.webContents.getURL(),
                title: target.view.webContents.getTitle(),
              }),
            ],
            { targetRef: target.targetRef },
          );
        }
        case "screenshot":
          return screenshot(entry);
        case "perform":
          return perform(entry, request);
        case "wait":
          return waitFor(entry, request);
        case "download":
          return download(entry, request.url);
        default:
          throw new BrowserHostError(
            "invalid_action",
            `Unsupported browser action '${action}'.`,
          );
      }
    });
  }

  function requireBearer(request) {
    const authorization =
      typeof request.headers.authorization === "string"
        ? request.headers.authorization
        : "";
    const expected = Buffer.from(`Bearer ${bearerToken}`, "utf8");
    const actual = Buffer.from(authorization, "utf8");
    if (
      actual.length !== expected.length ||
      !crypto.timingSafeEqual(actual, expected)
    ) {
      throw new BrowserHostError(
        "unauthorized",
        "A valid bearer token is required.",
        401,
      );
    }
  }

  function sendJson(response, statusCode, value) {
    const body = Buffer.from(JSON.stringify(value), "utf8");
    if (body.length > MAX_RESPONSE_BYTES) {
      const error = Buffer.from(
        JSON.stringify({
          error: {
            code: "response_too_large",
            message: `Browser response exceeds the ${MAX_RESPONSE_BYTES} byte limit.`,
          },
        }),
        "utf8",
      );
      response.writeHead(413, {
        "Content-Type": "application/json; charset=utf-8",
        "Content-Length": error.length,
        "Cache-Control": "no-store",
      });
      response.end(error);
      return;
    }
    response.writeHead(statusCode, {
      "Content-Type": "application/json; charset=utf-8",
      "Content-Length": body.length,
      "Cache-Control": "no-store",
      "X-Content-Type-Options": "nosniff",
    });
    response.end(body);
  }

  async function readJsonBody(request) {
    const contentType = String(
      request.headers["content-type"] || "",
    ).toLowerCase();
    if (!contentType.startsWith("application/json")) {
      throw new BrowserHostError(
        "unsupported_media_type",
        "Content-Type must be application/json.",
        415,
      );
    }
    const chunks = [];
    let bytes = 0;
    for await (const chunk of request) {
      bytes += chunk.length;
      if (bytes > MAX_REQUEST_BYTES) {
        throw new BrowserHostError(
          "request_too_large",
          `Request body exceeds the ${MAX_REQUEST_BYTES} byte limit.`,
          413,
        );
      }
      chunks.push(chunk);
    }
    try {
      return JSON.parse(Buffer.concat(chunks).toString("utf8"));
    } catch {
      throw new BrowserHostError(
        "invalid_json",
        "Request body is not valid JSON.",
      );
    }
  }

  async function handleBrokerRequest(request, response) {
    try {
      requireBearer(request);
      const requestUrl = new URL(request.url || "/", "http://127.0.0.1");
      if (request.method === "GET" && requestUrl.pathname === "/health") {
        sendJson(response, 200, {
          ok: true,
          service: "opentopia-desktop-browser-broker",
          sessions: sessions.size,
        });
        return;
      }
      if (request.method === "POST" && requestUrl.pathname === "/v1/browser") {
        const body = await readJsonBody(request);
        const output = await executeAction(body);
        sendJson(response, 200, output);
        return;
      }
      throw new BrowserHostError(
        "not_found",
        "Broker endpoint was not found.",
        404,
      );
    } catch (error) {
      const serialized = serializeError(error);
      log(
        serialized.statusCode >= 500 ? "error" : "warn",
        "browser.broker.request.failed",
        {
          method: request.method,
          path: request.url,
          error: serialized,
        },
      );
      sendJson(response, serialized.statusCode, {
        error: {
          code: serialized.code,
          message: serialized.message,
          ...(serialized.details || {}),
        },
      });
    }
  }

  async function startBroker() {
    if (brokerServer && brokerUrl)
      return { url: brokerUrl, token: bearerToken };
    brokerServer = http.createServer((request, response) => {
      void handleBrokerRequest(request, response);
    });
    brokerServer.on("clientError", (_error, socket) => {
      socket.end("HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n");
    });
    brokerServer.requestTimeout = MAX_WAIT_MS + 5_000;
    brokerServer.headersTimeout = 10_000;
    await new Promise((resolve, reject) => {
      brokerServer.once("error", reject);
      brokerServer.listen(0, "127.0.0.1", () => {
        brokerServer.off("error", reject);
        resolve();
      });
    });
    const address = brokerServer.address();
    if (!address || typeof address === "string") {
      throw new Error("Browser broker did not bind to a TCP address.");
    }
    brokerUrl = `http://127.0.0.1:${address.port}`;
    log("info", "browser.broker.started", { url: brokerUrl });
    return { url: brokerUrl, token: bearerToken };
  }

  function registerIpc(ipcMain) {
    const handle = (channel, handler) => {
      ipcMain.handle(channel, async (event, ...args) => {
        assertRenderer(event);
        return handler(...args);
      });
    };
    handle(IPC_CHANNELS.create, async (options = {}) => {
      const entry = createSession(options.sessionId, options);
      if (options.bounds) setBounds(entry, options.bounds);
      if (options.visible !== undefined) setVisibility(entry, options.visible);
      if (options.url) {
        await runExclusive(entry, () =>
          navigate(entry, options.url, parseWaitOptions(options)),
        );
      }
      return sessionState(entry);
    });
    handle(IPC_CHANNELS.destroy, (sessionId) => destroySession(sessionId));
    handle(IPC_CHANNELS.getState, (sessionId) =>
      sessionState(requireSession(sessionId)),
    );
    handle(IPC_CHANNELS.navigate, (sessionId, url) => {
      const entry = requireSession(sessionId);
      return runExclusive(entry, () => {
        beginUserControl(entry);
        return navigate(entry, url, null);
      });
    });
    handle(IPC_CHANNELS.navigateFromAddressBar, (sessionId, url) =>
      navigateFromAddressBar(sessionId, url),
    );
    handle(IPC_CHANNELS.beginUserControl, (sessionId) => {
      const entry = requireSession(sessionId);
      return runExclusive(entry, () => beginUserControl(entry));
    });
    handle(IPC_CHANNELS.back, async (sessionId) => {
      const entry = requireSession(sessionId);
      return runExclusive(entry, async () => {
        beginUserControl(entry);
        const history = navigationHistory(entry.view.webContents);
        if (history.canGoBack()) history.goBack();
        return sessionState(entry);
      });
    });
    handle(IPC_CHANNELS.forward, async (sessionId) => {
      const entry = requireSession(sessionId);
      return runExclusive(entry, async () => {
        beginUserControl(entry);
        const history = navigationHistory(entry.view.webContents);
        if (history.canGoForward()) history.goForward();
        return sessionState(entry);
      });
    });
    handle(IPC_CHANNELS.reload, async (sessionId) => {
      const entry = requireSession(sessionId);
      return runExclusive(entry, async () => {
        beginUserControl(entry);
        entry.view.webContents.reload();
        return sessionState(entry);
      });
    });
    handle(IPC_CHANNELS.setBounds, (sessionId, bounds) =>
      setBounds(requireSession(sessionId), bounds),
    );
    handle(IPC_CHANNELS.setVisibility, (sessionId, visible) =>
      setVisibility(requireSession(sessionId), visible),
    );
    handle(IPC_CHANNELS.show, (sessionId, bounds) => {
      const entry = requireSession(sessionId);
      if (bounds) setBounds(entry, bounds);
      return setVisibility(entry, true);
    });
    handle(IPC_CHANNELS.hide, (sessionId) =>
      setVisibility(requireSession(sessionId), false),
    );
  }

  function removeWindowListeners() {
    for (const [window, eventName, listener] of windowListeners) {
      window.removeListener(eventName, listener);
    }
    windowListeners = [];
  }

  function attachWindow(window) {
    removeWindowListeners();
    attachedWindow = window;
    windowSuspended = window.isMinimized() || !window.isVisible();
    for (const entry of sessions.values()) attachEntry(entry);

    const suspend = () => {
      windowSuspended = true;
      for (const entry of sessions.values()) {
        setActualVisibility(entry);
        emitState(entry);
      }
    };
    const resume = () => {
      windowSuspended = false;
      for (const entry of sessions.values()) {
        attachEntry(entry);
        setActualVisibility(entry);
        emitState(entry);
      }
    };
    const closed = () => {
      removeWindowListeners();
      attachedWindow = null;
      windowSuspended = true;
      destroyAllSessions();
    };
    const rendererNavigationStarted = (
      _event,
      _url,
      _isInPlace,
      isMainFrame,
    ) => {
      if (!isMainFrame) return;
      for (const entry of sessions.values()) {
        entry.requestedVisible = false;
        setActualVisibility(entry);
        emitState(entry);
      }
    };
    for (const eventName of ["minimize", "hide"]) {
      window.on(eventName, suspend);
      windowListeners.push([window, eventName, suspend]);
    }
    for (const eventName of ["restore", "show"]) {
      window.on(eventName, resume);
      windowListeners.push([window, eventName, resume]);
    }
    window.webContents.on("did-start-navigation", rendererNavigationStarted);
    windowListeners.push([
      window.webContents,
      "did-start-navigation",
      rendererNavigationStarted,
    ]);
    window.once("closed", closed);
    windowListeners.push([window, "closed", closed]);
  }

  async function close() {
    removeWindowListeners();
    destroyAllSessions();
    attachedWindow = null;
    if (brokerServer) {
      const server = brokerServer;
      brokerServer = null;
      brokerUrl = null;
      server.closeAllConnections?.();
      await new Promise((resolve) => server.close(resolve));
    }
  }

  return {
    attachWindow,
    close,
    executeAction,
    navigateFromAddressBar,
    registerIpc,
    startBroker,
  };
}

module.exports = {
  IPC_CHANNELS,
  createDesktopBrowserHost,
};
