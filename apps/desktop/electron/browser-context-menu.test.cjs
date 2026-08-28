const assert = require("node:assert/strict");
const test = require("node:test");
const {
  createBrowserContextMenuTemplate,
  popupBrowserContextMenu,
} = require("./browser-context-menu.cjs");

function actions(overrides = {}) {
  return {
    back() {},
    canGoBack: true,
    canGoForward: false,
    copyImage() {},
    copyText() {},
    edit() {},
    forward() {},
    inspect() {},
    openNewTab() {},
    reload() {},
    saveResource() {},
    searchSelection() {},
    ...overrides,
  };
}

function ids(template) {
  return template.filter((item) => item.id).map((item) => item.id);
}

test("builds link actions and rejects non-HTTP link protocols", () => {
  assert.deepEqual(
    ids(
      createBrowserContextMenuTemplate(
        {
          linkURL: "https://example.test/docs",
          mediaType: "none",
          suggestedFilename: "docs.html",
        },
        actions(),
      ),
    ),
    [
      "open-link-new-tab",
      "copy-link-address",
      "save-link-as",
      "inspect-element",
    ],
  );
  assert.deepEqual(
    ids(
      createBrowserContextMenuTemplate(
        { linkURL: "javascript:alert(1)", mediaType: "none" },
        actions(),
      ),
    ),
    ["back", "forward", "reload", "inspect-element"],
  );
  assert.deepEqual(
    ids(
      createBrowserContextMenuTemplate(
        {
          linkURL: "https://user:secret@example.test/private",
          mediaType: "none",
        },
        actions(),
      ),
    ),
    ["back", "forward", "reload", "inspect-element"],
  );
});

test("still copies data-backed images without exposing an unsafe source URL", () => {
  assert.deepEqual(
    ids(
      createBrowserContextMenuTemplate(
        {
          hasImageContents: true,
          mediaType: "image",
          srcURL: "data:image/png;base64,AAAA",
        },
        actions(),
      ),
    ),
    ["copy-image", "inspect-element"],
  );
});

test("combines image and enclosing-link actions without duplicate separators", () => {
  const template = createBrowserContextMenuTemplate(
    {
      hasImageContents: true,
      linkURL: "https://example.test/article",
      mediaType: "image",
      srcURL: "https://cdn.example.test/photo.png",
      x: 14,
      y: 28,
    },
    actions(),
  );
  assert.deepEqual(ids(template), [
    "open-link-new-tab",
    "copy-link-address",
    "save-link-as",
    "open-image-new-tab",
    "copy-image",
    "copy-image-address",
    "save-image-as",
    "inspect-element",
  ]);
  assert.equal(template.filter((item) => item.type === "separator").length, 2);
});

test("builds media-specific actions for video", () => {
  assert.deepEqual(
    ids(
      createBrowserContextMenuTemplate(
        {
          mediaType: "video",
          srcURL: "https://media.example.test/demo.mp4",
        },
        actions(),
      ),
    ),
    [
      "open-video-new-tab",
      "copy-video-address",
      "save-video-as",
      "inspect-element",
    ],
  );
});

test("uses edit flags to enable editable commands", () => {
  const template = createBrowserContextMenuTemplate(
    {
      editFlags: {
        canCopy: true,
        canCut: false,
        canDelete: true,
        canPaste: true,
        canRedo: false,
        canSelectAll: true,
        canUndo: true,
      },
      isEditable: true,
      mediaType: "none",
    },
    actions(),
  );
  assert.equal(template.find((item) => item.id === "edit-undo").enabled, true);
  assert.equal(template.find((item) => item.id === "edit-cut").enabled, false);
  assert.equal(template.find((item) => item.id === "edit-paste").enabled, true);
  assert.equal(template.find((item) => item.id === "edit-redo").enabled, false);
});

test("copies and searches a bounded selection label", () => {
  const selected =
    "a deliberately long selection that should not make the menu too wide";
  const calls = [];
  const template = createBrowserContextMenuTemplate(
    { mediaType: "none", selectionText: selected },
    actions({
      copyText: (value) => calls.push(["copy", value]),
      searchSelection: (value) => calls.push(["search", value]),
    }),
  );
  const search = template.find((item) => item.id === "search-selection");
  assert.equal(search.label, "使用 Google 搜索“a deliberately long sele…”");
  template.find((item) => item.id === "copy-selection").click();
  search.click();
  assert.deepEqual(calls, [
    ["copy", selected],
    ["search", selected],
  ]);
});

test("shows page navigation only for a plain page context", () => {
  const template = createBrowserContextMenuTemplate(
    { mediaType: "none" },
    actions(),
  );
  assert.deepEqual(ids(template), [
    "back",
    "forward",
    "reload",
    "inspect-element",
  ]);
  assert.equal(template.find((item) => item.id === "back").enabled, true);
  assert.equal(template.find((item) => item.id === "forward").enabled, false);
});

test("pops up the native menu for the supplied window", () => {
  const calls = [];
  const owner = { id: "main-window" };
  const Menu = {
    buildFromTemplate(template) {
      calls.push(["build", ids(template)]);
      return {
        popup(options) {
          calls.push(["popup", options]);
        },
      };
    },
  };
  assert.equal(
    popupBrowserContextMenu(Menu, owner, { mediaType: "none" }, actions()),
    true,
  );
  assert.deepEqual(calls, [
    ["build", ["back", "forward", "reload", "inspect-element"]],
    ["popup", { window: owner }],
  ]);
});
