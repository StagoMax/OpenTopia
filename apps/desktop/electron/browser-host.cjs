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
const MAX_INTERACTIVE_ELEMENTS = 500;
const MAX_OBSERVATIONS_PER_SESSION = 12;
const OBSERVATION_TTL_MS = 120_000;
const MAX_NODE_POSITION_DRIFT = 24;
const DEFAULT_BACKGROUND_BOUNDS = Object.freeze({
  x: 0,
  y: 0,
  width: 1280,
  height: 800,
});
const SESSION_ID_PATTERN = /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/;
const ALLOWED_PROTOCOLS = new Set(["http:", "https:"]);

class BrowserHostError extends Error {
  constructor(code, message, statusCode = 400) {
    super(message);
    this.name = "BrowserHostError";
    this.code = code;
    this.statusCode = statusCode;
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

function timeoutAfter(milliseconds, label) {
  return new Promise((_, reject) => {
    const timer = setTimeout(() => {
      reject(
        new BrowserHostError(
          "timeout",
          `${label} timed out after ${milliseconds} ms.`,
          504,
        ),
      );
    }, milliseconds);
    timer.unref?.();
  });
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
    };
  }
  return {
    code: "browser_host_error",
    message: error?.message || String(error),
    statusCode: 500,
  };
}

function createDesktopBrowserHost(options) {
  const { app, WebContentsView, getMainWindow, logger = () => {} } = options;
  const sessions = new Map();
  const partitionNonce = crypto.randomBytes(16).toString("hex");
  const bearerToken = crypto.randomBytes(32).toString("base64url");
  let brokerServer = null;
  let brokerUrl = null;
  let attachedWindow = null;
  let windowSuspended = false;
  let windowListeners = [];

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

  function configureRemoteContents(entry) {
    const webContents = entry.view.webContents;
    const browserSession = webContents.session;
    browserSession.setPermissionRequestHandler(
      (_contents, _permission, callback) => callback(false),
    );
    browserSession.setPermissionCheckHandler(() => false);

    webContents.setWindowOpenHandler(() => ({ action: "deny" }));
    webContents.on("will-attach-webview", (event) => event.preventDefault());
    webContents.on("will-navigate", (event, targetUrl) => {
      const navigationUrl = event?.url || targetUrl;
      try {
        const normalized = normalizeUrl(navigationUrl);
        const host = new URL(normalized).hostname.toLowerCase();
        if (!entry.allowedNavigationHosts.has(host)) {
          throw new BrowserHostError(
            "unapproved_navigation_host",
            `Navigation to '${host}' requires an explicit address-bar or Agent navigation.`,
            403,
          );
        }
      } catch (error) {
        event.preventDefault();
        entry.lastError = serializeError(error).message;
        emitState(entry);
        log("warn", "browser.navigation.blocked", {
          sessionId: entry.sessionId,
          url: navigationUrl,
          error: serializeError(error),
        });
      }
    });
    webContents.on("will-redirect", (event, targetUrl) => {
      const navigationUrl = event?.url || targetUrl;
      try {
        const normalized = normalizeUrl(navigationUrl);
        const host = new URL(normalized).hostname.toLowerCase();
        if (!entry.allowedNavigationHosts.has(host)) {
          throw new BrowserHostError(
            "unapproved_redirect_host",
            `Redirect to '${host}' requires a separate approved navigation.`,
            403,
          );
        }
      } catch (error) {
        event.preventDefault();
        entry.lastError = serializeError(error).message;
        emitState(entry);
        log("warn", "browser.redirect.blocked", {
          sessionId: entry.sessionId,
          url: navigationUrl,
          error: serializeError(error),
        });
      }
    });

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

    browserSession.on("will-download", (event, item, sourceContents) => {
      if (sourceContents !== webContents || !entry.pendingDownload) {
        event.preventDefault();
        return;
      }
      const pending = entry.pendingDownload;
      entry.pendingDownload = null;
      pending.accept(item);
    });
  }

  function createSession(sessionId) {
    const normalized = normalizeSessionId(sessionId);
    const existing = sessions.get(normalized);
    if (existing && !existing.destroyed) return existing;
    if (sessions.size >= MAX_SESSIONS) {
      throw new BrowserHostError(
        "too_many_sessions",
        `At most ${MAX_SESSIONS} browser sessions may be active.`,
        429,
      );
    }

    const partitionHash = crypto
      .createHash("sha256")
      .update(`${partitionNonce}:${normalized}`)
      .digest("hex")
      .slice(0, 24);
    const view = new WebContentsView({
      webPreferences: {
        partition: `opentopia-browser-${partitionHash}`,
        nodeIntegration: false,
        contextIsolation: true,
        sandbox: true,
        webSecurity: true,
        allowRunningInsecureContent: false,
        spellcheck: false,
      },
    });
    view.setBounds(DEFAULT_BACKGROUND_BOUNDS);
    const entry = {
      sessionId: normalized,
      view,
      bounds: { ...DEFAULT_BACKGROUND_BOUNDS },
      requestedVisible: false,
      attached: false,
      destroyed: false,
      pendingDownload: null,
      activeDownloadItem: null,
      allowedNavigationHosts: new Set(),
      lastError: null,
      observations: new Map(),
      queue: Promise.resolve(),
    };
    sessions.set(normalized, entry);
    configureRemoteContents(entry);
    attachEntry(entry);
    emitState(entry);
    log("info", "browser.session.created", { sessionId: normalized });
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
      return await operation();
    } finally {
      release();
    }
  }

  async function navigate(entry, rawUrl, waitOptions) {
    const targetUrl = normalizeUrl(rawUrl);
    entry.allowedNavigationHosts.add(new URL(targetUrl).hostname.toLowerCase());
    entry.lastError = null;
    const webContents = entry.view.webContents;
    const load = webContents.loadURL(targetUrl);
    await Promise.race([load, timeoutAfter(MAX_WAIT_MS, "Navigation")]);
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

  async function snapshot(entry) {
    const webContents = entry.view.webContents;
    const result = await webContents.executeJavaScript(
      `(() => {
        const byteLimit = ${MAX_SNAPSHOT_BYTES};
        const elementLimit = ${MAX_INTERACTIVE_ELEMENTS};
        const encoder = new TextEncoder();
        const truncate = (value, limit) => {
          const text = String(value || "");
          if (encoder.encode(text).length <= limit) return { value: text, truncated: false };
          let low = 0;
          let high = text.length;
          while (low < high) {
            const middle = Math.ceil((low + high) / 2);
            if (encoder.encode(text.slice(0, middle)).length <= limit) low = middle;
            else high = middle - 1;
          }
          return { value: text.slice(0, low), truncated: true };
        };
        const escape = (value) => window.CSS && CSS.escape
          ? CSS.escape(String(value))
          : String(value).replace(/[^a-zA-Z0-9_-]/g, (char) => "\\\\" + char);
        const selectorFor = (element) => {
          if (element.id) return "#" + escape(element.id);
          const parts = [];
          let current = element;
          while (current && current.nodeType === Node.ELEMENT_NODE && parts.length < 8) {
            let part = current.localName || "*";
            const siblings = current.parentElement
              ? Array.from(current.parentElement.children).filter((item) => item.localName === current.localName)
              : [];
            if (siblings.length > 1) part += ":nth-of-type(" + (siblings.indexOf(current) + 1) + ")";
            parts.unshift(part);
            current = current.parentElement;
          }
          return parts.join(" > ");
        };
        const candidates = Array.from(document.querySelectorAll(
          "a[href],button,input,textarea,select,[role=button],[role=link],[contenteditable=true],[tabindex]"
        )).slice(0, elementLimit);
        const roleFor = (element) => element.getAttribute("role") || ({
          a: "link", button: "button", textarea: "textbox", select: "combobox",
          input: element.type === "checkbox" ? "checkbox" : element.type === "radio" ? "radio" : "textbox"
        })[element.localName] || element.localName;
        const interactiveElements = candidates
          .filter((element) => !element.disabled && element.getClientRects().length)
          .map((element) => {
            const rect = element.getBoundingClientRect();
            return {
              selector: selectorFor(element),
              tagName: element.localName,
              role: roleFor(element),
              name: truncate(element.innerText || element.value || element.getAttribute("aria-label") || element.getAttribute("placeholder") || "", 2048).value,
              href: element.href || null,
              formAction: element.getAttribute("formaction") || (element.form && element.form.getAttribute("action")) || null,
              editable: Boolean(element.isContentEditable || (["input", "textarea", "select"].includes(element.localName) && !element.readOnly)),
              bounds: { x: rect.x, y: rect.y, width: rect.width, height: rect.height }
            };
          });
        const body = truncate(document.body ? document.body.innerText : "", byteLimit);
        return {
          url: document.location.href,
          title: document.title,
          text: body.value,
          text_truncated: body.truncated,
          interactive_elements: interactiveElements,
          interactive_elements_truncated: candidates.length >= elementLimit
        };
      })()`,
      false,
    );
    const text = truncateUtf8(result?.text || "", MAX_SNAPSHOT_BYTES);
    const value = {
      url: result?.url || webContents.getURL(),
      title: result?.title || webContents.getTitle(),
      text: text.value,
      textTruncated: Boolean(result?.text_truncated || text.truncated),
      interactiveElements: Array.isArray(result?.interactive_elements)
        ? result.interactive_elements.slice(0, MAX_INTERACTIVE_ELEMENTS)
        : [],
    };
    return browserOutput(
      webContents,
      "snapshot",
      [
        { type: "text", text: value.text, truncated: value.textTruncated },
        jsonContent(value),
      ],
      {
        title: value.title,
        interactive_elements_truncated: Boolean(
          result?.interactive_elements_truncated,
        ),
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
    const output = await snapshot(entry);
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
      if (!raw || typeof raw.selector !== "string") continue;
      const nodeRef = crypto.randomUUID();
      const node = {
        nodeRef,
        role: String(raw.role || raw.tagName || "element"),
        name: String(raw.name || ""),
        tagName: String(raw.tagName || ""),
        bounds: raw.bounds || { x: 0, y: 0, width: 0, height: 0 },
        href: typeof raw.href === "string" ? raw.href : null,
        formAction: typeof raw.formAction === "string" ? raw.formAction : null,
        editable: Boolean(raw.editable),
      };
      nodes.push(node);
      bindings.set(nodeRef, { node, selector: raw.selector });
    }
    entry.observations.set(observationId, {
      capturedAt: Date.now(),
      url: String(snapshotValueResult.url || entry.view.webContents.getURL()),
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
      title: String(snapshotValueResult.title || entry.view.webContents.getTitle()),
      text: String(snapshotValueResult.text || ""),
      textTruncated: Boolean(snapshotValueResult.textTruncated),
      nodes,
      screenshot: screenshotValue,
    };
  }

  function observedNode(entry, rawObservationId, rawNodeRef) {
    if (typeof rawObservationId !== "string" || typeof rawNodeRef !== "string") {
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
      expected.href === current.href &&
      expected.formAction === current.formAction &&
      expected.editable === current.editable &&
      Math.abs(Number(bounds.x) - Number(currentBounds.x)) <= MAX_NODE_POSITION_DRIFT &&
      Math.abs(Number(bounds.y) - Number(currentBounds.y)) <= MAX_NODE_POSITION_DRIFT &&
      Math.abs(Number(bounds.width) - Number(currentBounds.width)) <= MAX_NODE_POSITION_DRIFT &&
      Math.abs(Number(bounds.height) - Number(currentBounds.height)) <= MAX_NODE_POSITION_DRIFT
    );
  }

  async function perform(entry, request) {
    const { observation, binding } = observedNode(
      entry,
      request.observationId,
      request.nodeRef,
    );
    const webContents = entry.view.webContents;
    if (webContents.getURL() !== observation.url) {
      throw staleObservation("The page URL changed after the observation.");
    }
    const output = await snapshot(entry);
    const current = snapshotValue(output)?.interactiveElements?.find(
      (node) => node?.selector === binding.selector,
    );
    if (!current) {
      throw staleObservation("The observed element no longer exists.");
    }
    const currentNode = {
      ...current,
      nodeRef: binding.node.nodeRef,
      href: typeof current.href === "string" ? current.href : null,
      formAction: typeof current.formAction === "string" ? current.formAction : null,
      editable: Boolean(current.editable),
    };
    if (!nodesMatch(binding.node, currentNode)) {
      throw staleObservation("The observed element changed or moved.");
    }
    if (request.operation === "click") {
      await click(entry, binding.selector);
    } else if (request.operation === "type") {
      if (!currentNode.editable) {
        throw staleObservation("The observed element is no longer editable.");
      }
      await typeText(entry, binding.selector, request.text);
    } else {
      throw new BrowserHostError("invalid_action", "operation must be click or type.");
    }
    return {
      observationId: request.observationId,
      nodeRef: request.nodeRef,
      action: request.operation,
      target: currentNode,
      url: webContents.getURL(),
      title: webContents.getTitle(),
    };
  }

  async function withDebugger(webContents, operation) {
    const browserDebugger = webContents.debugger;
    let attachedHere = false;
    if (!browserDebugger.isAttached()) {
      browserDebugger.attach("1.3");
      attachedHere = true;
    }
    try {
      return await operation(browserDebugger);
    } finally {
      if (attachedHere && browserDebugger.isAttached())
        browserDebugger.detach();
    }
  }

  async function screenshot(entry) {
    const webContents = entry.view.webContents;
    const result = await withDebugger(webContents, (browserDebugger) =>
      browserDebugger.sendCommand("Page.captureScreenshot", {
        format: "png",
        fromSurface: true,
        captureBeyondViewport: false,
      }),
    );
    if (!result?.data) {
      throw new BrowserHostError(
        "screenshot_failed",
        "Page.captureScreenshot returned no image data.",
        500,
      );
    }
    const bytes = Buffer.from(result.data, "base64");
    if (bytes.length > MAX_SCREENSHOT_BYTES) {
      throw new BrowserHostError(
        "screenshot_too_large",
        `Screenshot is ${bytes.length} bytes; maximum is ${MAX_SCREENSHOT_BYTES}.`,
        413,
      );
    }
    return browserOutput(webContents, "screenshot", [
      { type: "image", mime_type: "image/png", bytes: Array.from(bytes) },
    ]);
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
    const downloadDirectory = path.join(
      app.getPath("temp"),
      "opentopia-browser-downloads",
      crypto
        .createHash("sha256")
        .update(entry.sessionId)
        .digest("hex")
        .slice(0, 16),
    );
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
    if (!entry.view.webContents.isDestroyed()) entry.view.webContents.close();
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
      "navigate",
      "snapshot",
      "observe",
      "observation_node",
      "screenshot",
      "perform",
      "wait",
      "download",
      "close",
    ]);
    if (!supported.has(action)) {
      throw new BrowserHostError(
        "invalid_action",
        `Unsupported browser action '${action}'.`,
      );
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
        case "navigate":
          return navigate(entry, request.url, parseWaitOptions(request));
        case "snapshot":
          return snapshot(entry);
        case "observe":
          return observe(entry, Boolean(request.includeScreenshot));
        case "observation_node":
          return observedNode(entry, request.observationId, request.nodeRef)
            .binding.node;
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
        error: { code: serialized.code, message: serialized.message },
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
      const entry = createSession(options.sessionId);
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
      return runExclusive(entry, () => navigate(entry, url, null));
    });
    handle(IPC_CHANNELS.back, async (sessionId) => {
      const entry = requireSession(sessionId);
      return runExclusive(entry, async () => {
        const history = navigationHistory(entry.view.webContents);
        if (history.canGoBack()) history.goBack();
        return sessionState(entry);
      });
    });
    handle(IPC_CHANNELS.forward, async (sessionId) => {
      const entry = requireSession(sessionId);
      return runExclusive(entry, async () => {
        const history = navigationHistory(entry.view.webContents);
        if (history.canGoForward()) history.goForward();
        return sessionState(entry);
      });
    });
    handle(IPC_CHANNELS.reload, async (sessionId) => {
      const entry = requireSession(sessionId);
      return runExclusive(entry, async () => {
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
    registerIpc,
    startBroker,
  };
}

module.exports = {
  IPC_CHANNELS,
  createDesktopBrowserHost,
};
