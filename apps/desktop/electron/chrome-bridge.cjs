const crypto = require("node:crypto");
const http = require("node:http");
const { URL } = require("node:url");

const PROTOCOL_VERSION = 1;
const EXTENSION_ID = "epamencadbkagkjllnlggdhldeepobdm";
const EXTENSION_ORIGIN = `chrome-extension://${EXTENSION_ID}`;
const PORT_FIRST = 32191;
const PORT_LAST = 32206;
const MAX_REQUEST_BYTES = 1024 * 1024;
const MAX_EVENTS = 1024;
const COMMAND_TIMEOUT_MS = 35_000;
const LONG_POLL_MS = 25_000;
const PAIRING_TTL_MS = 5 * 60_000;
const SESSION_ID_PATTERN = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

class ChromeBridgeError extends Error {
  constructor(code, message, statusCode = 400) {
    super(message);
    this.name = "ChromeBridgeError";
    this.code = code;
    this.statusCode = statusCode;
  }
}

function createChromeBridge({ logger = () => {}, onStateChanged = () => {} } = {}) {
  const backendToken = crypto.randomBytes(32).toString("base64url");
  const sessions = new Map();
  let server = null;
  let baseUrl = null;

  function log(level, event, metadata = {}) {
    try {
      logger(level, event, metadata);
    } catch {
      // Logging is not allowed to break the control plane.
    }
  }

  function normalizeSessionId(value) {
    if (typeof value !== "string" || !SESSION_ID_PATTERN.test(value)) {
      throw new ChromeBridgeError("invalid_session_id", "sessionId must be a UUID.");
    }
    return value.toLowerCase();
  }

  function publicState(entry) {
    const pairingActive =
      Boolean(entry.pairingCode) && entry.pairingExpiresAt > Date.now();
    return {
      sessionId: entry.sessionId,
      availability: "available",
      status: entry.tabId
        ? "attached"
        : entry.extensionToken
          ? "waiting_for_tab"
          : pairingActive
            ? "waiting_for_extension"
            : "idle",
      pairingCode: pairingActive ? entry.pairingCode : undefined,
      pairingExpiresAt: pairingActive
        ? new Date(entry.pairingExpiresAt).toISOString()
        : undefined,
      tabId: entry.tabId ?? undefined,
      targetId: entry.targetId ?? undefined,
      url: entry.url || "",
      title: entry.title || "",
      error: entry.error || null,
    };
  }

  function emitState(entry) {
    const state = publicState(entry);
    try {
      onStateChanged(state);
    } catch {
      // Renderer lifecycle must not affect bridge state.
    }
    return state;
  }

  function entryFor(sessionId, create = false) {
    const normalized = normalizeSessionId(sessionId);
    let entry = sessions.get(normalized);
    if (!entry && create) {
      entry = {
        sessionId: normalized,
        pairingCode: null,
        pairingExpiresAt: 0,
        extensionToken: null,
        extensionId: null,
        tabId: null,
        targetId: null,
        url: "",
        title: "",
        error: null,
        queue: [],
        pending: new Map(),
        commandWaiters: [],
        events: [],
        eventSequence: 0,
        eventWaiters: [],
      };
      sessions.set(normalized, entry);
    }
    if (!entry) {
      throw new ChromeBridgeError(
        "session_not_found",
        "Chrome session is not paired.",
        404,
      );
    }
    return entry;
  }

  function startPairing(sessionId) {
    const entry = entryFor(sessionId, true);
    detachEntry(entry, "A new Chrome pairing was requested.", false);
    entry.extensionToken = null;
    entry.extensionId = null;
    entry.events.length = 0;
    entry.eventSequence = 0;
    entry.pairingCode = String(crypto.randomInt(0, 1_000_000)).padStart(6, "0");
    entry.pairingExpiresAt = Date.now() + PAIRING_TTL_MS;
    entry.error = null;
    log("info", "chrome.bridge.pairing-started", { sessionId: entry.sessionId });
    return emitState(entry);
  }

  function getStatus(sessionId) {
    const normalized = normalizeSessionId(sessionId);
    const entry = sessions.get(normalized);
    return entry
      ? publicState(entry)
      : {
          sessionId: normalized,
          availability: baseUrl ? "available" : "unavailable",
          status: "idle",
          url: "",
          title: "",
          error: baseUrl ? null : "Chrome bridge is unavailable.",
        };
  }

  function rejectPending(entry, reason) {
    for (const pending of entry.pending.values()) {
      clearTimeout(pending.timeout);
      pending.reject(
        new ChromeBridgeError("chrome_detached", reason, 409),
      );
    }
    entry.pending.clear();
    entry.queue.length = 0;
    for (const wake of entry.commandWaiters.splice(0)) wake();
  }

  function detachEntry(entry, reason, exposeError = true) {
    rejectPending(entry, reason);
    entry.tabId = null;
    entry.targetId = null;
    entry.url = "";
    entry.title = "";
    entry.error = exposeError ? reason : null;
    emitState(entry);
  }

  async function disconnect(sessionId) {
    const entry = entryFor(sessionId);
    if (entry.tabId && entry.extensionToken) {
      try {
        await enqueueCommand(entry, "OpenTopia.detach", {}, null, 5_000);
      } catch {
        // Local state is authoritative when Chrome is already gone.
      }
    }
    detachEntry(entry, "Chrome tab was disconnected.", false);
    entry.extensionToken = null;
    entry.extensionId = null;
    entry.pairingCode = null;
    entry.pairingExpiresAt = 0;
    return emitState(entry);
  }

  function notifyCommand(entry) {
    const wake = entry.commandWaiters.shift();
    if (wake) wake();
  }

  function nextQueuedCommand(entry) {
    while (entry.queue.length) {
      const command = entry.queue.shift();
      if (entry.pending.has(command.requestId)) return command;
    }
    return null;
  }

  async function waitForCommand(entry, waitMs = LONG_POLL_MS) {
    const immediate = nextQueuedCommand(entry);
    if (immediate) return immediate;
    await new Promise((resolve) => {
      const timer = setTimeout(() => {
        const index = entry.commandWaiters.indexOf(wake);
        if (index >= 0) entry.commandWaiters.splice(index, 1);
        resolve();
      }, Math.min(Math.max(Number(waitMs) || 0, 0), LONG_POLL_MS));
      const wake = () => {
        clearTimeout(timer);
        resolve();
      };
      entry.commandWaiters.push(wake);
    });
    return nextQueuedCommand(entry);
  }

  function enqueueCommand(
    entry,
    method,
    params = {},
    targetSessionId = "root",
    timeoutMs = COMMAND_TIMEOUT_MS,
  ) {
    if (!entry.tabId || !entry.extensionToken) {
      return Promise.reject(
        new ChromeBridgeError(
          "chrome_tab_not_attached",
          "Select and authorize a Chrome tab first.",
          409,
        ),
      );
    }
    const requestId = crypto.randomUUID();
    const command = {
      protocolVersion: PROTOCOL_VERSION,
      requestId,
      type: "command",
      sessionId: entry.sessionId,
      tabId: entry.tabId,
      method,
      params,
      targetSessionId,
    };
    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        entry.pending.delete(requestId);
        reject(
          new ChromeBridgeError(
            "chrome_command_timeout",
            `Chrome command timed out: ${method}`,
            504,
          ),
        );
      }, Math.min(Math.max(timeoutMs, 1), COMMAND_TIMEOUT_MS));
      entry.pending.set(requestId, { method, resolve, reject, timeout });
      entry.queue.push(command);
      notifyCommand(entry);
    });
  }

  function completeCommand(entry, body) {
    const pending = entry.pending.get(body.requestId);
    if (!pending) {
      throw new ChromeBridgeError(
        "unknown_request_id",
        "Chrome command request is no longer pending.",
        409,
      );
    }
    entry.pending.delete(body.requestId);
    clearTimeout(pending.timeout);
    if (body.error) {
      pending.reject(
        new ChromeBridgeError(
          "cdp_error",
          String(body.error.message || body.error),
          422,
        ),
      );
    } else {
      pending.resolve(body.result ?? null);
    }
  }

  function recordEvent(entry, body) {
    const event = {
      seq: ++entry.eventSequence,
      method: String(body.method || ""),
      params:
        body.params && typeof body.params === "object" ? body.params : {},
      sessionId: body.targetSessionId || "root",
    };
    if (!event.method) {
      throw new ChromeBridgeError("invalid_event", "Event method is required.");
    }
    entry.events.push(event);
    if (entry.events.length > MAX_EVENTS) entry.events.shift();
    if (event.method === "Page.frameNavigated") {
      const frame = event.params.frame;
      if (frame && !frame.parentId && typeof frame.url === "string") {
        entry.url = frame.url;
        emitState(entry);
      }
    }
    for (const wake of entry.eventWaiters.splice(0)) wake();
  }

  async function eventsAfter(entry, after, waitMs) {
    const collect = () => entry.events.filter((event) => event.seq > after);
    let events = collect();
    if (events.length || waitMs <= 0) return events;
    await new Promise((resolve) => {
      const timer = setTimeout(() => {
        const index = entry.eventWaiters.indexOf(wake);
        if (index >= 0) entry.eventWaiters.splice(index, 1);
        resolve();
      }, Math.min(waitMs, LONG_POLL_MS));
      const wake = () => {
        clearTimeout(timer);
        resolve();
      };
      entry.eventWaiters.push(wake);
    });
    events = collect();
    return events;
  }

  function requireBackend(request) {
    const authorization = request.headers.authorization || "";
    const expected = `Bearer ${backendToken}`;
    const valid =
      authorization.length === expected.length &&
      crypto.timingSafeEqual(Buffer.from(authorization), Buffer.from(expected));
    if (!valid) throw new ChromeBridgeError("unauthorized", "Unauthorized.", 401);
  }

  function extensionEntry(request, requestUrl) {
    const origin = request.headers.origin;
    if (origin && origin !== EXTENSION_ORIGIN) {
      throw new ChromeBridgeError("forbidden_origin", "Extension origin is not trusted.", 403);
    }
    const token = (request.headers.authorization || "").replace(/^Bearer\s+/i, "");
    const sessionId = requestUrl.searchParams.get("sessionId");
    const entry = entryFor(sessionId);
    if (
      !entry.extensionToken ||
      token.length !== entry.extensionToken.length ||
      !crypto.timingSafeEqual(Buffer.from(token), Buffer.from(entry.extensionToken))
    ) {
      throw new ChromeBridgeError("unauthorized", "Extension session is unauthorized.", 401);
    }
    return entry;
  }

  function corsHeaders(request, discovery = false) {
    const origin = request.headers.origin;
    return {
      "Access-Control-Allow-Origin":
        discovery || origin === EXTENSION_ORIGIN ? origin || "*" : "null",
      "Access-Control-Allow-Headers": "authorization, content-type",
      "Access-Control-Allow-Methods": "GET, POST, OPTIONS",
      Vary: "Origin",
    };
  }

  function sendJson(response, statusCode, value, headers = {}) {
    const body = Buffer.from(JSON.stringify(value));
    response.writeHead(statusCode, {
      "Content-Type": "application/json; charset=utf-8",
      "Content-Length": body.length,
      "Cache-Control": "no-store",
      ...headers,
    });
    response.end(body);
  }

  async function readJson(request) {
    const chunks = [];
    let size = 0;
    for await (const chunk of request) {
      size += chunk.length;
      if (size > MAX_REQUEST_BYTES) {
        throw new ChromeBridgeError("request_too_large", "Request is too large.", 413);
      }
      chunks.push(chunk);
    }
    try {
      return JSON.parse(Buffer.concat(chunks).toString("utf8") || "{}");
    } catch {
      throw new ChromeBridgeError("invalid_json", "Request body is invalid JSON.");
    }
  }

  async function handleRequest(request, response) {
    const requestUrl = new URL(request.url || "/", "http://127.0.0.1");
    const discovery = requestUrl.pathname === "/v1/discovery";
    const cors = corsHeaders(request, discovery);
    if (request.method === "OPTIONS") {
      response.writeHead(204, cors);
      response.end();
      return;
    }

    if (request.method === "GET" && discovery) {
      sendJson(response, 200, {
        service: "opentopia-chrome-bridge",
        protocolVersion: PROTOCOL_VERSION,
        extensionId: EXTENSION_ID,
      }, cors);
      return;
    }

    if (requestUrl.pathname.startsWith("/v1/backend/")) {
      requireBackend(request);
      if (request.method === "GET" && requestUrl.pathname === "/v1/backend/health") {
        sendJson(response, 200, { ok: true, protocolVersion: PROTOCOL_VERSION });
        return;
      }
      const sessionMatch = requestUrl.pathname.match(/^\/v1\/backend\/sessions\/([^/]+)$/);
      if (request.method === "GET" && sessionMatch) {
        const entry = entryFor(sessionMatch[1]);
        if (!entry.tabId || !entry.targetId) {
          throw new ChromeBridgeError(
            "chrome_tab_not_attached",
            "Select and authorize a Chrome tab first.",
            409,
          );
        }
        sendJson(response, 200, {
          sessionId: entry.sessionId,
          tabId: entry.tabId,
          targetId: entry.targetId,
          url: entry.url,
          title: entry.title,
        });
        return;
      }
      if (request.method === "POST" && requestUrl.pathname === "/v1/backend/command") {
        const body = await readJson(request);
        const entry = entryFor(body.sessionId);
        if (typeof body.method !== "string" || !body.method) {
          throw new ChromeBridgeError("invalid_command", "Chrome command method is required.");
        }
        const result = await enqueueCommand(
          entry,
          body.method,
          body.params || {},
          body.targetSessionId ?? "root",
        );
        sendJson(response, 200, { result });
        return;
      }
      const eventsMatch = requestUrl.pathname.match(/^\/v1\/backend\/events\/([^/]+)$/);
      if (request.method === "GET" && eventsMatch) {
        const entry = entryFor(eventsMatch[1]);
        const after = Math.max(Number(requestUrl.searchParams.get("after")) || 0, 0);
        const waitMs = Math.max(Number(requestUrl.searchParams.get("waitMs")) || 0, 0);
        sendJson(response, 200, {
          events: await eventsAfter(entry, after, waitMs),
          attached: Boolean(entry.tabId),
        });
        return;
      }
    }

    if (request.method === "POST" && requestUrl.pathname === "/v1/extension/pair") {
      const body = await readJson(request);
      const origin = request.headers.origin;
      if (origin && origin !== EXTENSION_ORIGIN) {
        throw new ChromeBridgeError("forbidden_origin", "Extension origin is not trusted.", 403);
      }
      if (body.protocolVersion !== PROTOCOL_VERSION || body.extensionId !== EXTENSION_ID) {
        throw new ChromeBridgeError("protocol_mismatch", "Chrome extension is incompatible.", 409);
      }
      const entry = [...sessions.values()].find(
        (candidate) =>
          candidate.pairingCode === String(body.code || "") &&
          candidate.pairingExpiresAt > Date.now(),
      );
      if (!entry) {
        throw new ChromeBridgeError("invalid_pairing_code", "Pairing code is invalid or expired.", 401);
      }
      entry.extensionToken = crypto.randomBytes(32).toString("base64url");
      entry.extensionId = EXTENSION_ID;
      entry.pairingCode = null;
      entry.pairingExpiresAt = 0;
      entry.error = null;
      emitState(entry);
      sendJson(response, 200, {
        protocolVersion: PROTOCOL_VERSION,
        sessionId: entry.sessionId,
        token: entry.extensionToken,
      }, cors);
      return;
    }

    if (requestUrl.pathname.startsWith("/v1/extension/")) {
      const entry = extensionEntry(request, requestUrl);
      if (request.method === "POST" && requestUrl.pathname === "/v1/extension/attach") {
        const body = await readJson(request);
        if (!Number.isInteger(body.tabId) || body.tabId < 0 || typeof body.targetId !== "string") {
          throw new ChromeBridgeError("invalid_target", "Chrome target metadata is invalid.");
        }
        const conflict = [...sessions.values()].find(
          (candidate) => candidate !== entry && candidate.tabId === body.tabId,
        );
        if (conflict) {
          throw new ChromeBridgeError(
            "tab_already_owned",
            "This Chrome tab is already attached to another task.",
            409,
          );
        }
        entry.tabId = body.tabId;
        entry.targetId = body.targetId;
        entry.url = typeof body.url === "string" ? body.url : "";
        entry.title = typeof body.title === "string" ? body.title : "";
        entry.error = null;
        emitState(entry);
        sendJson(response, 200, publicState(entry), cors);
        return;
      }
      if (request.method === "POST" && requestUrl.pathname === "/v1/extension/state") {
        const body = await readJson(request);
        if (body.detached) {
          detachEntry(entry, String(body.reason || "Chrome ended the debugger session."));
        } else {
          if (typeof body.url === "string") entry.url = body.url;
          if (typeof body.title === "string") entry.title = body.title;
          emitState(entry);
        }
        sendJson(response, 200, { ok: true }, cors);
        return;
      }
      if (request.method === "GET" && requestUrl.pathname === "/v1/extension/next") {
        const command = await waitForCommand(
          entry,
          Number(requestUrl.searchParams.get("waitMs")) || LONG_POLL_MS,
        );
        sendJson(response, 200, { command }, cors);
        return;
      }
      if (request.method === "POST" && requestUrl.pathname === "/v1/extension/result") {
        const body = await readJson(request);
        completeCommand(entry, body);
        sendJson(response, 200, { ok: true }, cors);
        return;
      }
      if (request.method === "POST" && requestUrl.pathname === "/v1/extension/event") {
        recordEvent(entry, await readJson(request));
        sendJson(response, 200, { ok: true }, cors);
        return;
      }
    }

    throw new ChromeBridgeError("not_found", "Bridge endpoint was not found.", 404);
  }

  async function start() {
    if (server && baseUrl) return { url: baseUrl, token: backendToken };
    for (let port = PORT_FIRST; port <= PORT_LAST; port += 1) {
      const candidate = http.createServer((request, response) => {
        void handleRequest(request, response).catch((error) => {
          const normalized =
            error instanceof ChromeBridgeError
              ? error
              : new ChromeBridgeError("bridge_error", error?.message || String(error), 500);
          log(normalized.statusCode >= 500 ? "error" : "warn", "chrome.bridge.request-failed", {
            method: request.method,
            path: request.url,
            code: normalized.code,
          });
          if (!response.headersSent) {
            sendJson(response, normalized.statusCode, {
              error: { code: normalized.code, message: normalized.message },
            }, corsHeaders(request));
          } else {
            response.destroy();
          }
        });
      });
      candidate.requestTimeout = COMMAND_TIMEOUT_MS + 5_000;
      candidate.headersTimeout = 10_000;
      try {
        await new Promise((resolve, reject) => {
          candidate.once("error", reject);
          candidate.listen(port, "127.0.0.1", () => {
            candidate.off("error", reject);
            resolve();
          });
        });
        server = candidate;
        baseUrl = `http://127.0.0.1:${port}`;
        log("info", "chrome.bridge.started", { url: baseUrl, protocolVersion: PROTOCOL_VERSION });
        return { url: baseUrl, token: backendToken };
      } catch (error) {
        candidate.close();
        if (error?.code !== "EADDRINUSE" || port === PORT_LAST) throw error;
      }
    }
    throw new Error("No loopback port is available for the Chrome bridge.");
  }

  async function close() {
    for (const entry of sessions.values()) {
      rejectPending(entry, "OpenTopia is shutting down.");
    }
    sessions.clear();
    if (!server) return;
    const active = server;
    server = null;
    baseUrl = null;
    active.closeAllConnections?.();
    await new Promise((resolve) => active.close(resolve));
  }

  return {
    close,
    disconnect,
    getStatus,
    start,
    startPairing,
  };
}

module.exports = {
  EXTENSION_ID,
  PORT_FIRST,
  PORT_LAST,
  PROTOCOL_VERSION,
  createChromeBridge,
};
