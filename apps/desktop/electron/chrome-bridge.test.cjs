const assert = require("node:assert/strict");
const test = require("node:test");
const {
  EXTENSION_ID,
  PROTOCOL_VERSION,
  createChromeBridge,
} = require("./chrome-bridge.cjs");

async function jsonRequest(url, options = {}) {
  const response = await fetch(url, {
    ...options,
    headers: { Connection: "close", ...(options.headers || {}) },
  });
  const payload = await response.json();
  return { response, payload };
}

test("pairs, owns one tab, and correlates commands and events", async () => {
  const states = [];
  const bridge = createChromeBridge({
    onStateChanged: (state) => states.push(state),
  });
  const backend = await bridge.start();
  try {
    const sessionId = "11111111-1111-4111-8111-111111111111";
    const pairing = bridge.startPairing(sessionId);
    assert.match(pairing.pairingCode, /^\d{6}$/);

    const paired = await jsonRequest(`${backend.url}/v1/extension/pair`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Origin: `chrome-extension://${EXTENSION_ID}`,
      },
      body: JSON.stringify({
        protocolVersion: PROTOCOL_VERSION,
        extensionId: EXTENSION_ID,
        code: pairing.pairingCode,
      }),
    });
    assert.equal(paired.response.status, 200);
    const extensionHeaders = {
      Authorization: `Bearer ${paired.payload.token}`,
      "Content-Type": "application/json",
      Origin: `chrome-extension://${EXTENSION_ID}`,
    };
    const sessionQuery = `sessionId=${sessionId}`;

    const attached = await jsonRequest(
      `${backend.url}/v1/extension/attach?${sessionQuery}`,
      {
        method: "POST",
        headers: extensionHeaders,
        body: JSON.stringify({
          tabId: 42,
          targetId: "target-42",
          url: "https://example.test/",
          title: "Example",
        }),
      },
    );
    assert.equal(attached.payload.status, "attached");
    const extensionStatus = await jsonRequest(
      `${backend.url}/v1/extension/status?${sessionQuery}`,
      { headers: extensionHeaders },
    );
    assert.equal(extensionStatus.payload.status, "attached");
    assert.equal(extensionStatus.payload.tabId, 42);

    const commandResult = jsonRequest(`${backend.url}/v1/backend/command`, {
      method: "POST",
      headers: {
        Authorization: `Bearer ${backend.token}`,
        "Content-Type": "application/json",
      },
      body: JSON.stringify({
        sessionId,
        targetSessionId: "root",
        method: "Runtime.evaluate",
        params: { expression: "document.title" },
      }),
    });
    const next = await jsonRequest(
      `${backend.url}/v1/extension/next?${sessionQuery}&waitMs=1000`,
      { headers: extensionHeaders },
    );
    assert.equal(next.payload.command.method, "Runtime.evaluate");
    assert.equal(next.payload.command.targetSessionId, "root");

    await jsonRequest(`${backend.url}/v1/extension/result?${sessionQuery}`, {
      method: "POST",
      headers: extensionHeaders,
      body: JSON.stringify({
        requestId: next.payload.command.requestId,
        result: { result: { value: "Example" } },
      }),
    });
    assert.deepEqual((await commandResult).payload.result, {
      result: { value: "Example" },
    });

    const navigation = bridge.runUserAction(
      sessionId,
      "navigate",
      "https://example.test/next",
    );
    const navigationCommand = await jsonRequest(
      `${backend.url}/v1/extension/next?${sessionQuery}&waitMs=1000`,
      { headers: extensionHeaders },
    );
    assert.equal(navigationCommand.payload.command.method, "Page.navigate");
    assert.equal(
      navigationCommand.payload.command.params.url,
      "https://example.test/next",
    );
    await jsonRequest(`${backend.url}/v1/extension/result?${sessionQuery}`, {
      method: "POST",
      headers: extensionHeaders,
      body: JSON.stringify({
        requestId: navigationCommand.payload.command.requestId,
        result: { frameId: "frame-1" },
      }),
    });
    await navigation;

    await jsonRequest(`${backend.url}/v1/extension/event?${sessionQuery}`, {
      method: "POST",
      headers: extensionHeaders,
      body: JSON.stringify({
        method: "Page.loadEventFired",
        params: { timestamp: 1 },
        targetSessionId: "root",
      }),
    });
    const events = await jsonRequest(
      `${backend.url}/v1/backend/events/${sessionId}?after=0`,
      { headers: { Authorization: `Bearer ${backend.token}` } },
    );
    assert.deepEqual(events.payload.events[0], {
      seq: 1,
      method: "Page.loadEventFired",
      params: { timestamp: 1 },
      sessionId: "root",
    });
    assert.equal(states.at(-1).status, "attached");
  } finally {
    await bridge.close();
  }
});

test("rejects untrusted extension origins with a stable error response", async () => {
  const bridge = createChromeBridge();
  const backend = await bridge.start();
  try {
    const first = bridge.startPairing("22222222-2222-4222-8222-222222222222");
    const missingOrigin = await jsonRequest(
      `${backend.url}/v1/extension/pair`,
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          protocolVersion: PROTOCOL_VERSION,
          extensionId: EXTENSION_ID,
          code: first.pairingCode,
        }),
      },
    );
    assert.equal(missingOrigin.response.status, 403);
    const rejected = await jsonRequest(`${backend.url}/v1/extension/pair`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Origin: "https://example.test",
      },
      body: JSON.stringify({
        protocolVersion: PROTOCOL_VERSION,
        extensionId: EXTENSION_ID,
        code: first.pairingCode,
      }),
    });
    assert.equal(rejected.response.status, 403);
  } finally {
    await bridge.close();
  }
});

test("supports a release-specific trusted Chrome extension ID", async () => {
  const extensionId = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
  const bridge = createChromeBridge({ extensionId });
  const backend = await bridge.start();
  try {
    const discovery = await jsonRequest(`${backend.url}/v1/discovery`);
    assert.equal(discovery.payload.extensionId, extensionId);
  } finally {
    await bridge.close();
  }
  assert.throws(
    () => createChromeBridge({ extensionId: "not-an-extension-id" }),
    /extension ID is invalid/,
  );
});
