const assert = require("node:assert/strict");
const { EventEmitter } = require("node:events");
const test = require("node:test");
const {
  IPC_CHANNELS,
  createDesktopBrowserHost,
} = require("./browser-host.cjs");

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
    if (this.offsetCheckStale) return false;
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

  goToIndex(index) {
    this.calls.push(`index:${index}`);
    this.pendingIndex = index;
  }

  finishNavigation({ event = "did-navigate" } = {}) {
    assert.notEqual(this.pendingIndex, null, "expected a pending history move");
    this.activeIndex = this.pendingIndex;
    this.pendingIndex = null;
    const entry = this.entries[this.activeIndex];
    this.webContents.url = entry.url;
    this.webContents.title = entry.title;
    if (event === "did-stop-loading") {
      this.webContents.emit("did-stop-loading");
      return;
    }
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
    this.windowOpenHandler = null;
    this.pendingLoads = [];
    this.commands = [];
    this.downloadUrls = [];
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

  setWindowOpenHandler(handler) {
    this.windowOpenHandler = handler;
  }

  openWindow(url) {
    assert.ok(this.windowOpenHandler, "expected a window-open handler");
    return this.windowOpenHandler({ url });
  }

  loadURL(url) {
    this.url = url;
    return new Promise((resolve, reject) => {
      this.pendingLoads.push({ resolve, reject });
    });
  }

  finishLoad({ title = "Loaded" } = {}) {
    const load = this.pendingLoads.shift();
    assert.ok(load, "expected a pending page load");
    this.title = title;
    load.resolve();
  }

  failLoad(error = new Error("navigation failed")) {
    const load = this.pendingLoads.shift();
    assert.ok(load, "expected a pending page load");
    load.reject(error);
  }

  copyImageAt(x, y) {
    this.commands.push(["copy-image", x, y]);
  }

  downloadURL(url) {
    this.downloadUrls.push(url);
  }

  inspectElement(x, y) {
    this.commands.push(["inspect", x, y]);
  }

  paste() {
    this.commands.push(["paste"]);
  }

  reload() {
    this.commands.push(["reload"]);
  }

  close() {
    this.destroyed = true;
    this.emit("destroyed");
  }
}

function createHarness() {
  const views = [];
  const attachedViews = new Set();
  const clipboardWrites = [];
  const menus = [];
  const saveDialogs = [];
  class FakeWebContentsView {
    constructor(options = {}) {
      this.webContents = new FakeWebContents();
      this.webPreferences = options.webPreferences ?? {};
      this.visible = true;
      this.bounds = null;
      this.visibilityChanges = [];
      views.push(this);
    }

    setBounds(bounds) {
      this.bounds = { ...bounds };
    }

    setVisible(visible) {
      this.visible = visible;
      this.visibilityChanges.push(visible);
    }
  }

  const renderer = new FakeWebContents();
  const states = [];
  renderer.send = (channel, state) => states.push({ channel, state });
  const window = new EventEmitter();
  window.webContents = renderer;
  window.contentView = {
    addChildView(view) {
      assert.equal(
        view.visible,
        false,
        "browser views must be hidden before they are attached",
      );
      attachedViews.add(view);
    },
    removeChildView(view) {
      attachedViews.delete(view);
    },
  };
  window.isDestroyed = () => false;
  window.isMinimized = () => false;
  window.isVisible = () => true;

  const host = createDesktopBrowserHost({
    app: { getPath: () => "downloads" },
    Menu: {
      buildFromTemplate(template) {
        const menu = { template, popupOptions: null };
        menus.push(menu);
        return {
          popup(options) {
            menu.popupOptions = options;
          },
        };
      },
    },
    WebContentsView: FakeWebContentsView,
    clipboard: {
      writeText(value) {
        clipboardWrites.push(value);
      },
    },
    dialog: {
      async showSaveDialog(window, options) {
        saveDialogs.push({ options, window });
        return { canceled: false, filePath: "downloads/saved-resource.html" };
      },
    },
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
    window,
    attachedViews,
    clipboardWrites,
    menus,
    saveDialogs,
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
    assert.deepEqual(history.calls, ["index:0"]);
    assert.equal(backSettled, false);
    assert.equal(
      (await invoke(IPC_CHANNELS.getState, sessionId)).url,
      initial.url,
    );

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
    assert.deepEqual(history.calls, ["index:0", "index:1"]);
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

test("visible browser session handles mouse back and forward app commands", async () => {
  const { host, invoke, views, window } = createHarness();
  const sessionId = "browser:mouse-history-test";
  try {
    await invoke(IPC_CHANNELS.create, { sessionId, visible: false });
    await invoke(IPC_CHANNELS.show, sessionId, {
      x: 320,
      y: 80,
      width: 640,
      height: 480,
    });
    const history = views[0].webContents.navigationHistory;
    const event = {
      preventDefaultCalls: 0,
      preventDefault() {
        this.preventDefaultCalls += 1;
      },
    };

    window.emit("app-command", event, "browser-backward");
    await waitFor(() => history.calls.length === 1);
    assert.equal(event.preventDefaultCalls, 1);
    assert.deepEqual(history.calls, ["index:0"]);
    history.finishNavigation();

    window.emit("app-command", event, "browser-forward");
    await waitFor(() => history.calls.length === 2);
    assert.equal(event.preventDefaultCalls, 2);
    assert.deepEqual(history.calls, ["index:0", "index:1"]);
    history.finishNavigation();
  } finally {
    await host.close();
  }
});

test("history index keeps navigation available when Electron's offset check is stale", async () => {
  const { host, invoke, views } = createHarness();
  const sessionId = "browser:history-index-stale-test";
  try {
    await invoke(IPC_CHANNELS.create, { sessionId, visible: true });
    const history = views[0].webContents.navigationHistory;
    history.offsetCheckStale = true;

    const state = await invoke(IPC_CHANNELS.getState, sessionId);
    assert.equal(state.canGoBack, true);
    assert.equal(state.canGoForward, true);

    const back = invoke(IPC_CHANNELS.back, sessionId);
    await waitFor(() => history.calls.length === 1);
    assert.deepEqual(history.calls, ["index:0"]);
    history.finishNavigation();

    const afterBack = await back;
    assert.equal(afterBack.url, "https://example.test/one");
    assert.equal(afterBack.canGoBack, false);
    assert.equal(afterBack.canGoForward, true);
  } finally {
    await host.close();
  }
});

test("state publishes the page favicon after the active view reports one", async () => {
  const { host, invoke, views } = createHarness();
  const sessionId = "browser:favicon-state-test";
  try {
    await invoke(IPC_CHANNELS.create, { sessionId, visible: false });
    views[0].webContents.emit("page-favicon-updated", {}, [
      "https://example.test/favicon.ico",
    ]);

    const state = await invoke(IPC_CHANNELS.getState, sessionId);
    assert.equal(state.faviconUrl, "https://example.test/favicon.ico");
  } finally {
    await host.close();
  }
});

test("recorded page navigations provide history when Electron exposes none", async () => {
  const { host, invoke, views } = createHarness();
  const sessionId = "browser:fallback-history-test";
  try {
    await invoke(IPC_CHANNELS.create, { sessionId, visible: false });
    const webContents = views[0].webContents;

    for (const [url, title] of [
      ["https://example.test/one", "One"],
      ["https://example.test/two", "Two"],
    ]) {
      webContents.url = url;
      webContents.title = title;
      webContents.emit("did-navigate", {}, url, 200, "OK");
    }
    webContents.navigationHistory = {};

    const beforeBack = await invoke(IPC_CHANNELS.getState, sessionId);
    assert.equal(beforeBack.canGoBack, true);
    assert.equal(beforeBack.canGoForward, false);

    const back = invoke(IPC_CHANNELS.back, sessionId);
    await waitFor(() => webContents.pendingLoads.length === 1);
    webContents.finishLoad({ title: "One" });
    webContents.emit("did-navigate", {}, webContents.url, 200, "OK");
    const afterBack = await back;
    assert.equal(afterBack.url, "https://example.test/one");
    assert.equal(afterBack.canGoBack, false);
    assert.equal(afterBack.canGoForward, true);

    const forward = invoke(IPC_CHANNELS.forward, sessionId);
    await waitFor(() => webContents.pendingLoads.length === 1);
    webContents.finishLoad({ title: "Two" });
    webContents.emit("did-navigate", {}, webContents.url, 200, "OK");
    const afterForward = await forward;
    assert.equal(afterForward.url, "https://example.test/two");
    assert.equal(afterForward.canGoBack, true);
    assert.equal(afterForward.canGoForward, false);
  } finally {
    await host.close();
  }
});

test("back and forward switch between owned popup targets", async () => {
  const { host, invoke, views } = createHarness();
  const sessionId = "browser:target-history-test";
  try {
    await invoke(IPC_CHANNELS.create, {
      sessionId,
      visible: true,
      bounds: { x: 320, y: 80, width: 640, height: 480 },
    });
    const opener = views[0].webContents;

    opener.openWindow("https://baidu.test/");
    await waitFor(
      () =>
        views.length === 2 && views[1].webContents.pendingLoads.length === 1,
    );
    const popup = views[1].webContents;
    popup.finishLoad({ title: "Baidu" });
    popup.emit("did-navigate", {}, popup.url, 200, "OK");
    await waitFor(() => views[1].visible);

    const beforeBack = await invoke(IPC_CHANNELS.getState, sessionId);
    assert.equal(beforeBack.canGoBack, true);

    const afterBack = await invoke(IPC_CHANNELS.back, sessionId);
    assert.equal(afterBack.url, opener.url);
    assert.equal(afterBack.canGoForward, true);

    const afterForward = await invoke(IPC_CHANNELS.forward, sessionId);
    assert.equal(afterForward.url, popup.url);
  } finally {
    await host.close();
  }
});

test("history navigation completes when a back-forward cache restore only stops loading", async () => {
  const { host, invoke, views } = createHarness();
  const sessionId = "browser:history-cache-restore-test";
  try {
    await invoke(IPC_CHANNELS.create, { sessionId, visible: false });
    const history = views[0].webContents.navigationHistory;

    const back = invoke(IPC_CHANNELS.back, sessionId);
    await waitFor(() => history.calls.length === 1);
    history.finishNavigation({ event: "did-stop-loading" });

    const afterBack = await back;
    assert.equal(afterBack.url, "https://example.test/one");
    assert.equal(afterBack.canGoBack, false);
    assert.equal(afterBack.canGoForward, true);
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
    assert.deepEqual(history.calls, ["index:0"]);
    history.finishNavigation();
    const afterBack = await back;
    assert.equal(afterBack.url, "https://example.test/app");
    assert.equal(afterBack.canGoBack, false);
    assert.equal(afterBack.canGoForward, true);
  } finally {
    await host.close();
  }
});

test("only the most recently shown browser session is visible", async () => {
  const { attachedViews, host, invoke, views } = createHarness();
  try {
    await invoke(IPC_CHANNELS.create, {
      sessionId: "browser:tab:first",
      visible: false,
    });
    await invoke(IPC_CHANNELS.create, {
      sessionId: "browser:tab:second",
      visible: false,
    });

    assert.equal(views[0].visible, false);
    assert.equal(views[1].visible, false);
    assert.equal(attachedViews.size, 0);

    const firstBounds = { x: 320, y: 80, width: 640, height: 480 };
    await invoke(IPC_CHANNELS.show, "browser:tab:first", firstBounds);
    assert.equal(views[0].visible, true);
    assert.deepEqual(views[0].bounds, firstBounds);
    assert.deepEqual([...attachedViews], [views[0]]);

    const secondBounds = { x: 360, y: 96, width: 600, height: 440 };
    await invoke(IPC_CHANNELS.show, "browser:tab:second", secondBounds);
    assert.equal(views[0].visible, false);
    assert.equal(views[1].visible, true);
    assert.deepEqual(views[1].bounds, secondBounds);
    assert.deepEqual([...attachedViews], [views[1]]);

    assert.equal(
      (await invoke(IPC_CHANNELS.getState, "browser:tab:first")).visible,
      false,
    );
    assert.equal(
      (await invoke(IPC_CHANNELS.getState, "browser:tab:second")).visible,
      true,
    );
  } finally {
    await host.close();
  }
});

test("browser views keep background throttling enabled for tabs and popups", async () => {
  const { host, invoke, views } = createHarness();
  const sessionId = "browser:background-throttling-test";
  try {
    await invoke(IPC_CHANNELS.create, { sessionId, visible: false });
    assert.equal(views[0].webPreferences.backgroundThrottling, true);

    views[0].webContents.openWindow("https://example.test/popup");
    await waitFor(() => views.length === 2);
    assert.equal(views[1].webPreferences.backgroundThrottling, true);
  } finally {
    await host.close();
  }
});

test("a browser view cannot cover the app before its panel bounds are measured", async () => {
  const { attachedViews, host, invoke, views } = createHarness();
  const sessionId = "browser:unmeasured-visible-test";
  try {
    const initial = await invoke(IPC_CHANNELS.create, {
      sessionId,
      visible: true,
    });

    assert.equal(initial.visible, false);
    assert.equal(views[0].visible, false);
    assert.equal(attachedViews.size, 0);

    const bounds = { x: 320, y: 80, width: 640, height: 480 };
    const measured = await invoke(IPC_CHANNELS.setBounds, sessionId, bounds);
    assert.equal(measured.visible, true);
    assert.equal(views[0].visible, true);
    assert.deepEqual(views[0].bounds, bounds);

    await invoke(IPC_CHANNELS.hide, sessionId);
    assert.equal(views[0].visible, false);
    assert.equal(attachedViews.size, 0);
  } finally {
    await host.close();
  }
});

test("a Baidu target-blank result requests a new app browser tab", async () => {
  const { attachedViews, host, invoke, states, views } = createHarness();
  const sessionId = "browser:tab:baidu-result-test";
  try {
    const bounds = { x: 320, y: 80, width: 640, height: 480 };
    await invoke(IPC_CHANNELS.create, { sessionId, visible: false });
    await invoke(IPC_CHANNELS.show, sessionId, bounds);
    const opener = views[0];
    opener.webContents.url = "https://www.google.com/search?q=baidu";
    opener.webContents.title = "baidu - Google Search";

    assert.deepEqual(opener.webContents.openWindow("https://www.baidu.com/"), {
      action: "deny",
    });
    await waitFor(() =>
      states.some(({ channel }) => channel === IPC_CHANNELS.newTabRequested),
    );

    assert.equal(views.length, 1);
    assert.equal(opener.visible, true);
    assert.deepEqual([...attachedViews], [opener]);
    assert.deepEqual(
      states.find(({ channel }) => channel === IPC_CHANNELS.newTabRequested),
      {
        channel: IPC_CHANNELS.newTabRequested,
        state: {
          openerSessionId: sessionId,
          url: "https://www.baidu.com/",
        },
      },
    );
  } finally {
    await host.close();
  }
});

test("a link context menu opens a new app tab and copies its address", async () => {
  const { clipboardWrites, host, invoke, menus, states, views, window } =
    createHarness();
  const sessionId = "browser:tab:context-menu-link-test";
  try {
    await invoke(IPC_CHANNELS.create, { sessionId, visible: false });
    views[0].webContents.emit(
      "context-menu",
      {},
      {
        linkURL: "https://www.baidu.com/",
        mediaType: "none",
        suggestedFilename: "baidu.html",
        x: 48,
        y: 72,
      },
    );

    assert.equal(menus.length, 1);
    assert.equal(menus[0].popupOptions.window, window);
    const copyLink = menus[0].template.find(
      (item) => item.id === "copy-link-address",
    );
    const openLink = menus[0].template.find(
      (item) => item.id === "open-link-new-tab",
    );
    assert.ok(copyLink);
    assert.ok(openLink);
    copyLink.click();
    openLink.click();

    assert.deepEqual(clipboardWrites, ["https://www.baidu.com/"]);
    assert.deepEqual(
      states.find(
        ({ channel, state }) =>
          channel === IPC_CHANNELS.newTabRequested &&
          state.openerSessionId === sessionId,
      ),
      {
        channel: IPC_CHANNELS.newTabRequested,
        state: {
          openerSessionId: sessionId,
          url: "https://www.baidu.com/",
        },
      },
    );
  } finally {
    await host.close();
  }
});

test("a context-menu save uses the chosen path for the requested resource", async () => {
  const { host, invoke, menus, saveDialogs, views, window } = createHarness();
  const sessionId = "browser:tab:context-menu-save-test";
  try {
    await invoke(IPC_CHANNELS.create, { sessionId, visible: false });
    const webContents = views[0].webContents;
    webContents.emit(
      "context-menu",
      {},
      {
        linkURL: "https://example.test/report",
        mediaType: "none",
        suggestedFilename: "report.html",
        x: 24,
        y: 36,
      },
    );
    menus[0].template.find((item) => item.id === "save-link-as").click();
    await waitFor(() => webContents.downloadUrls.length === 1);

    assert.equal(saveDialogs.length, 1);
    assert.equal(saveDialogs[0].window, window);
    assert.match(saveDialogs[0].options.defaultPath, /report\.html$/);
    assert.deepEqual(webContents.downloadUrls, ["https://example.test/report"]);

    let savePath = null;
    const item = new EventEmitter();
    item.setSavePath = (value) => {
      savePath = value;
    };
    webContents.session.emit("will-download", {}, item, webContents);
    assert.match(savePath, /saved-resource\.html$/);
  } finally {
    await host.close();
  }
});

test("a shared-browser popup remains hidden until it loads, then becomes active", async () => {
  const { attachedViews, host, invoke, views } = createHarness();
  const sessionId = "browser:popup-load-test";
  try {
    const bounds = { x: 320, y: 80, width: 640, height: 480 };
    await invoke(IPC_CHANNELS.create, { sessionId, visible: false });
    await invoke(IPC_CHANNELS.show, sessionId, bounds);
    const opener = views[0];
    opener.webContents.url = "https://www.google.com/search?q=baidu";
    opener.webContents.title = "baidu - Google Search";

    assert.deepEqual(opener.webContents.openWindow("https://www.baidu.com/"), {
      action: "deny",
    });
    await waitFor(
      () =>
        views.length === 2 && views[1].webContents.pendingLoads.length === 1,
    );
    const popup = views[1];

    assert.equal(opener.visible, true);
    assert.equal(popup.visible, false);
    assert.deepEqual([...attachedViews], [opener]);
    assert.equal(
      (await invoke(IPC_CHANNELS.getState, sessionId)).url,
      opener.webContents.url,
    );

    popup.webContents.finishLoad({ title: "Baidu" });
    await waitFor(() => popup.visible);

    assert.equal(opener.visible, false);
    assert.equal(popup.visible, true);
    assert.deepEqual([...attachedViews], [popup]);
    assert.deepEqual(popup.bounds, bounds);
    const state = await invoke(IPC_CHANNELS.getState, sessionId);
    assert.equal(state.url, "https://www.baidu.com/");
    assert.equal(state.title, "Baidu");
  } finally {
    await host.close();
  }
});

test("a failed popup load keeps the opener visible and discards the blank target", async () => {
  const { attachedViews, host, invoke, views } = createHarness();
  const sessionId = "browser:popup-failure-test";
  try {
    await invoke(IPC_CHANNELS.create, {
      sessionId,
      visible: true,
      bounds: { x: 320, y: 80, width: 640, height: 480 },
    });
    const opener = views[0];

    opener.webContents.openWindow("https://baidu.test/");
    await waitFor(
      () =>
        views.length === 2 && views[1].webContents.pendingLoads.length === 1,
    );
    const popup = views[1];
    popup.webContents.failLoad();
    await waitFor(() => popup.webContents.destroyed);

    assert.equal(opener.visible, true);
    assert.equal(popup.visible, false);
    assert.deepEqual([...attachedViews], [opener]);
    assert.equal(
      (await invoke(IPC_CHANNELS.getState, sessionId)).url,
      opener.webContents.url,
    );
  } finally {
    await host.close();
  }
});

test("hide is idempotent after a browser session has already been destroyed", async () => {
  const { host, invoke } = createHarness();
  const sessionId = "browser:destroyed-hide-test";
  try {
    await invoke(IPC_CHANNELS.create, { sessionId, visible: true });
    await invoke(IPC_CHANNELS.destroy, sessionId);

    assert.deepEqual(await invoke(IPC_CHANNELS.hide, sessionId), {
      sessionId,
      visible: false,
      missing: true,
    });
  } finally {
    await host.close();
  }
});
