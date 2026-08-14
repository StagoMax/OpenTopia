const PROTOCOL_VERSION = 1;
const PORT_FIRST = 32191;
const PORT_LAST = 32206;
const POLL_ALARM = "opentopia-bridge-poll";
const FORWARDED_DEBUGGER_EVENTS = new Set([
  "Target.attachedToTarget",
  "Target.detachedFromTarget",
  "Target.targetInfoChanged",
  "Page.frameNavigated",
  "Page.lifecycleEvent",
  "Page.loadEventFired",
  "Page.navigatedWithinDocument",
  "Page.javascriptDialogOpening",
  "Network.loadingFailed",
]);

let polling = false;

function debuggerCall(method, ...args) {
  return new Promise((resolve, reject) => {
    chrome["debugger"][method](...args, (result) => {
      const error = chrome.runtime.lastError;
      if (error) reject(new Error(error.message));
      else resolve(result);
    });
  });
}

async function bridgeRequest(path, options = {}) {
  const state = await chrome.storage.local.get([
    "bridgeUrl",
    "sessionId",
    "token",
  ]);
  if (!state.bridgeUrl) throw new Error("OpenTopia bridge is not paired.");
  const headers = {
    "Content-Type": "application/json",
    ...(options.headers || {}),
  };
  if (state.token) headers.Authorization = `Bearer ${state.token}`;
  const separator = path.includes("?") ? "&" : "?";
  const sessionPath = state.sessionId
    ? `${path}${separator}sessionId=${encodeURIComponent(state.sessionId)}`
    : path;
  const response = await fetch(`${state.bridgeUrl}${sessionPath}`, {
    ...options,
    headers,
    cache: "no-store",
  });
  const payload = await response.json().catch(() => ({}));
  if (!response.ok)
    throw new Error(payload.error?.message || `Bridge HTTP ${response.status}`);
  return payload;
}

async function discoverBridge() {
  for (let port = PORT_FIRST; port <= PORT_LAST; port += 1) {
    const url = `http://127.0.0.1:${port}`;
    try {
      const response = await fetch(`${url}/v1/discovery`, {
        cache: "no-store",
      });
      const payload = await response.json();
      if (
        response.ok &&
        payload.service === "opentopia-chrome-bridge" &&
        payload.protocolVersion === PROTOCOL_VERSION &&
        payload.extensionId === chrome.runtime.id
      ) {
        return url;
      }
    } catch {
      // Keep scanning the small reserved loopback range.
    }
  }
  throw new Error("OpenTopia 桌面端尚未运行。");
}

async function pair(code) {
  const bridgeUrl = await discoverBridge();
  const response = await fetch(`${bridgeUrl}/v1/extension/pair`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      protocolVersion: PROTOCOL_VERSION,
      extensionId: chrome.runtime.id,
      code,
    }),
  });
  const payload = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(payload.error?.message || "配对失败。");
  const previous = await chrome.storage.local.get(["tabId"]);
  if (previous.tabId) {
    await debuggerCall("detach", { tabId: previous.tabId }).catch(() => {});
  }
  await chrome.storage.local.remove("tabId");
  await chrome.storage.local.set({
    bridgeUrl,
    sessionId: payload.sessionId,
    token: payload.token,
  });
  ensurePolling();
  return payload;
}

async function attachActiveTab() {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  if (
    !tab?.id ||
    !tab.url ||
    /^(chrome|edge|devtools|chrome-extension):/i.test(tab.url)
  ) {
    throw new Error("当前 Chrome 页面不允许连接。");
  }
  const state = await chrome.storage.local.get([
    "bridgeUrl",
    "sessionId",
    "token",
    "tabId",
  ]);
  if (!state.token || !state.sessionId)
    throw new Error("请先与 OpenTopia 配对。");
  if (state.tabId && state.tabId !== tab.id) {
    await debuggerCall("detach", { tabId: state.tabId }).catch(() => {});
  }
  await debuggerCall("attach", { tabId: tab.id }, "1.3");
  const targets = await debuggerCall("getTargets");
  const target = targets.find((candidate) => candidate.tabId === tab.id);
  if (!target?.id) {
    await debuggerCall("detach", { tabId: tab.id }).catch(() => {});
    throw new Error("Chrome 未提供所选标签页的调试目标。");
  }
  await chrome.storage.local.set({ tabId: tab.id });
  try {
    await bridgeRequest("/v1/extension/attach", {
      method: "POST",
      body: JSON.stringify({
        tabId: tab.id,
        targetId: target.id,
        url: tab.url,
        title: tab.title || "",
      }),
    });
  } catch (error) {
    await debuggerCall("detach", { tabId: tab.id }).catch(() => {});
    await chrome.storage.local.remove("tabId");
    throw error;
  }
  ensurePolling();
  return { tabId: tab.id, url: tab.url, title: tab.title || "" };
}

async function detach(
  reason = "Chrome debugger was detached.",
  clearPairing = false,
) {
  const state = await chrome.storage.local.get(["tabId"]);
  if (state.tabId)
    await debuggerCall("detach", { tabId: state.tabId }).catch(() => {});
  await chrome.storage.local.remove("tabId");
  await bridgeRequest("/v1/extension/state", {
    method: "POST",
    body: JSON.stringify({ detached: true, reason }),
  }).catch(() => {});
  if (clearPairing) {
    await chrome.storage.local.remove(["bridgeUrl", "sessionId", "token"]);
  }
}

async function executeCommand(command) {
  const state = await chrome.storage.local.get(["tabId"]);
  if (!state.tabId || state.tabId !== command.tabId) {
    throw new Error("The authorized Chrome tab is no longer attached.");
  }
  if (command.method === "OpenTopia.detach") {
    await detach("OpenTopia ended the session.", true);
    return {};
  }
  const target = { tabId: state.tabId };
  if (command.targetSessionId && command.targetSessionId !== "root") {
    target.sessionId = command.targetSessionId;
  }
  return debuggerCall(
    "sendCommand",
    target,
    command.method,
    command.params || {},
  );
}

async function poll() {
  if (polling) return;
  polling = true;
  try {
    while (true) {
      const state = await chrome.storage.local.get([
        "sessionId",
        "token",
        "tabId",
      ]);
      if (!state.sessionId || !state.token || !state.tabId) return;
      const { command } = await bridgeRequest(
        "/v1/extension/next?waitMs=25000",
      );
      if (!command) continue;
      try {
        const result = await executeCommand(command);
        await bridgeRequest("/v1/extension/result", {
          method: "POST",
          body: JSON.stringify({ requestId: command.requestId, result }),
        });
      } catch (error) {
        await bridgeRequest("/v1/extension/result", {
          method: "POST",
          body: JSON.stringify({
            requestId: command.requestId,
            error: { message: error?.message || String(error) },
          }),
        }).catch(() => {});
      }
    }
  } catch {
    // The alarm and popup will restart polling after transient disconnects.
  } finally {
    polling = false;
  }
}

function ensurePolling() {
  chrome.alarms.create(POLL_ALARM, { periodInMinutes: 0.5 });
  void poll();
}

async function currentStatus() {
  const state = await chrome.storage.local.get([
    "bridgeUrl",
    "sessionId",
    "token",
    "tabId",
  ]);
  if (!state.bridgeUrl || !state.sessionId || !state.token) return {};
  try {
    const remote = await bridgeRequest("/v1/extension/status");
    if (remote.status !== "attached" && state.tabId) {
      await chrome.storage.local.remove("tabId");
      delete state.tabId;
    }
    return state;
  } catch {
    if (state.tabId) {
      await debuggerCall("detach", { tabId: state.tabId }).catch(() => {});
    }
    await chrome.storage.local.remove([
      "bridgeUrl",
      "sessionId",
      "token",
      "tabId",
    ]);
    return {};
  }
}

async function reconcileStoredAttachment() {
  const state = await currentStatus();
  if (!state.tabId) return;
  try {
    await debuggerCall(
      "sendCommand",
      { tabId: state.tabId },
      "Runtime.evaluate",
      { expression: "void 0" },
    );
    ensurePolling();
  } catch {
    await chrome.storage.local.remove("tabId");
    await bridgeRequest("/v1/extension/state", {
      method: "POST",
      body: JSON.stringify({
        detached: true,
        reason: "Chrome restarted or released the debugger session.",
      }),
    }).catch(() => {});
  }
}

chrome.debugger.onEvent.addListener((source, method, params) => {
  void chrome.storage.local
    .get(["tabId"])
    .then(async (state) => {
      if (source.tabId !== state.tabId) return;
      if (method === "Page.javascriptDialogOpening") {
        const target = { tabId: state.tabId };
        if (source.sessionId) target.sessionId = source.sessionId;
        await debuggerCall(
          "sendCommand",
          target,
          "Page.handleJavaScriptDialog",
          {
            accept: false,
          },
        ).catch(() => {});
      }
      if (!FORWARDED_DEBUGGER_EVENTS.has(method)) return;
      await bridgeRequest("/v1/extension/event", {
        method: "POST",
        body: JSON.stringify({
          method,
          params,
          targetSessionId: source.sessionId || "root",
        }),
      });
    })
    .catch(() => {});
});

chrome.debugger.onDetach.addListener((source, reason) => {
  void chrome.storage.local.get(["tabId"]).then(async (state) => {
    if (source.tabId !== state.tabId) return;
    await chrome.storage.local.remove("tabId");
    await bridgeRequest("/v1/extension/state", {
      method: "POST",
      body: JSON.stringify({ detached: true, reason }),
    }).catch(() => {});
  });
});

chrome.tabs.onUpdated.addListener((tabId, changeInfo, tab) => {
  void chrome.storage.local
    .get(["tabId"])
    .then((state) => {
      if (tabId !== state.tabId) return;
      if (
        !changeInfo.url &&
        !changeInfo.title &&
        changeInfo.status !== "complete"
      )
        return;
      return bridgeRequest("/v1/extension/state", {
        method: "POST",
        body: JSON.stringify({ url: tab.url || "", title: tab.title || "" }),
      });
    })
    .catch(() => {});
});

chrome.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name === POLL_ALARM) void poll();
});

chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  const operation =
    message?.type === "pair"
      ? pair(String(message.code || ""))
      : message?.type === "attach"
        ? attachActiveTab()
        : message?.type === "detach"
          ? detach("User disconnected the tab.")
          : message?.type === "status"
            ? currentStatus()
            : Promise.reject(new Error("Unsupported extension request."));
  operation
    .then((value) => sendResponse({ ok: true, value }))
    .catch((error) => {
      sendResponse({ ok: false, error: error?.message || String(error) });
    });
  return true;
});

void reconcileStoredAttachment();

ensurePolling();
