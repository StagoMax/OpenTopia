const assert = require("node:assert/strict");
const { EventEmitter } = require("node:events");
const test = require("node:test");
const { IPC_CHANNELS, createDesktopBrowserHost } = require("./browser-host.cjs");

class FakeNavigationHistory {
  constructor(webContents) {
    this.webContents = webContents;
    this.entries = [
      { url: "https://example.test/one", title: "One" },
      { url: "https://example.test/two", title: "Two" },
      { url: "https://example.test/three", title: "Three" },
    ];
    this.activeIndex = 1;
    this.calls = [];
    this.pendingIndex = null;
  }

  canGoToOffset(offset) {
    // Model the Electron 41 behavior for same-document history: the legacy
    // direction helpers can be stale while the offset API is authoritative.
    return this.entries[this.activeIndex + offset] !== undefined;
  }

  canGoBack() {
    if (this.sameDocumentEntry) return false;
    return this.activeIndex > 0;
  }

  canGoForward() {
    if (this.sameDocumentEntry) return false;
    return this.activeIndex < this.entries.length - 1;
  }

  getActiveIndex() {
    return this.activeIndex;
  }

  getEntryAtIndex(index) {
    return this.entries[index] ?? null;
  }

  goBack() {
    this.calls.push("back");
    this.pendingIndex = this.activeIndex - 1;
  }

  goForward() {
    this.calls.push("forward");
    this.pendingIndex = this.activeIndex + 1;
  }

  goToOffset(offset) {
    this.calls.push(offset < 0 ? "back-offset" : "forward-offset");
    this.pendingIndex = this.activeIndex + offset;
  }

  finishNavigation() {
    assert.notEqual(this.pendingIndex, null, "expected a pending history move");
    this.activeIndex = this.pendingIndex;
    this.pendingIndex = null;
    const entry = this.entries[this.activeIndex];
    this.webContents.url = entry.url;
    this.webContents.title = entry.title;
    if (this.sameDocumentEntry) {
      this.webContents.emit("did-navigate-in-page", {}, entry.url, true);
    } else {
      this.webContents.emit("did-navigate", {}, entry.url, 200, "OK");
    }
  }
}

let nextWebContentsId = 1;

class FakeWebContents extends EventEmitter {
  constructor() {
    super();
    this.id = nextWebContentsId++;
    this.url = "https://example.test/two";
    this.title = "Two";
    this.destroyed = false;
    this.mainFrame = {};
    this.session = new EventEmitter();
    this.session.webRequest = { onBeforeRequest() {} };
    this.debugger = {
      isAttached: () => true,
      on() {},
      sendCommand: async () => {},
    };
    this.navigationHistory = new FakeNavigationHistory(this);
  }

  isDestroyed() {
    return this.destroyed;
  }

  isLoading() {
    return false;
  }

  getURL() {
    return this.url;
  }

  getTitle() {
    return this.title;
  }

  setWindowOpenHandler() {}

  close() {
    this.destroyed = true;
    this.emit("destroyed");
  }
}

function createHarness() {
  const views = [];
  class FakeWebContentsView {
    constructor() {
      this.webContents = new FakeWebContents();
      views.push(this);
    }

    setBounds() {}

    setVisible() {}
  }

  const renderer = new FakeWebContents();
  const states = [];
  renderer.send = (channel, state) => states.push({ channel, state });
  const window = new EventEmitter();
  window.webContents = renderer;
  window.contentView = {
    addChildView() {},
    removeChildView() {},
  };
  window.isDestroyed = () => false;
  window.isMinimized = () => false;
  window.isVisible = () => true;

  const host = createDesktopBrowserHost({
    app: { getPath: () => "downloads" },
    WebContentsView: FakeWebContentsView,
    nativeImage: null,
    getMainWindow: () => window,
  });
  host.attachWindow(window);

  const handlers = new Map();
  host.registerIpc({
    handle(channel, handler) {
      handlers.set(channel, handler);
    },
  });

  return {
    host,
    states,
    views,
    invoke(channel, ...args) {
      const handler = handlers.get(channel);
      assert.ok(handler, `missing IPC handler for ${channel}`);
      return handler(
        { sender: renderer, senderFrame: renderer.mainFrame },
        ...args,
      );
    },
  };
}

async function waitFor(predicate) {
  for (let attempt = 0; attempt < 20; attempt += 1) {
    if (predicate()) return;
    await new Promise((resolve) => setImmediate(resolve));
  }
  assert.fail("timed out waiting for browser history operation");
}

test("back and forward resolve only after the corresponding history navigation completes", async () => {
  const { host, invoke, states, views } = createHarness();
  const sessionId = "browser:history-test";
  try {
    const initial = await invoke(IPC_CHANNELS.create, {
      sessionId,
      visible: false,
    });
    const history = views[0].webContents.navigationHistory;
    assert.equal(initial.url, "https://example.test/two");
    assert.equal(initial.canGoBack, true);
    assert.equal(initial.canGoForward, true);

    let backSettled = false;
    let backError = null;
    const back = invoke(IPC_CHANNELS.back, sessionId)
      .then((state) => {
        backSettled = true;
        return state;
      })
      .catch((error) => {
        backError = error;
        return null;
      });
    await waitFor(() => history.calls.length === 1);
    assert.deepEqual(history.calls, ["back-offset"]);
    assert.equal(backSettled, false);
    assert.equal((await invoke(IPC_CHANNELS.getState, sessionId)).url, initial.url);

    views[0].webContents.emit("did-navigate", {}, "https://example.test/three");
    views[0].webContents.emit(
      "did-fail-load",
      {},
      -2,
      "ERR_FAILED",
      "https://example.test/three",
      true,
    );
    await new Promise((resolve) => setImmediate(resolve));
    assert.equal(backSettled, false);
    assert.equal(backError, null);

    history.finishNavigation();
    const afterBack = await back;
    assert.ok(afterBack);
    assert.equal(afterBack.url, "https://example.test/one");
    assert.equal(afterBack.canGoBack, false);
    assert.equal(afterBack.canGoForward, true);

    let forwardSettled = false;
    const forward = invoke(IPC_CHANNELS.forward, sessionId).then((state) => {
      forwardSettled = true;
      return state;
    });
    await waitFor(() => history.calls.length === 2);
    assert.deepEqual(history.calls, ["back-offset", "forward-offset"]);
    assert.equal(forwardSettled, false);

    history.finishNavigation();
    const afterForward = await forward;
    assert.equal(afterForward.url, "https://example.test/two");
    assert.equal(afterForward.canGoBack, true);
    assert.equal(afterForward.canGoForward, true);
    assert.deepEqual(states.at(-1), {
      channel: IPC_CHANNELS.state,
      state: afterForward,
    });
  } finally {
    await host.close();
  }
});

test("same-document history remains navigable when legacy direction checks are stale", async () => {
  const { host, invoke, views } = createHarness();
  const sessionId = "browser:same-document-history-test";
  try {
    await invoke(IPC_CHANNELS.create, { sessionId, visible: false });
    const history = views[0].webContents.navigationHistory;
    history.sameDocumentEntry = true;
    history.entries = [
      { url: "https://example.test/app", title: "App" },
      { url: "https://example.test/app#settings", title: "Settings" },
    ];
    history.activeIndex = 1;
    views[0].webContents.url = history.entries[1].url;
    views[0].webContents.title = history.entries[1].title;

    const state = await invoke(IPC_CHANNELS.getState, sessionId);
    assert.equal(state.canGoBack, true);
    assert.equal(state.canGoForward, false);

    const back = invoke(IPC_CHANNELS.back, sessionId);
    await waitFor(() => history.calls.length === 1);
    assert.deepEqual(history.calls, ["back-offset"]);
    history.finishNavigation();
    const afterBack = await back;
    assert.equal(afterBack.url, "https://example.test/app");
    assert.equal(afterBack.canGoBack, false);
    assert.equal(afterBack.canGoForward, true);
  } finally {
    await host.close();
  }
});
