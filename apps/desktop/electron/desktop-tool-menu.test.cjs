const assert = require("node:assert/strict");
const test = require("node:test");
const {
  TOOL_MENU_ACTIONS,
  createDesktopToolMenuTemplate,
  normalizeDesktopToolMenuRequest,
} = require("./desktop-tool-menu.cjs");

test("normalizes the bounded desktop tool menu request", () => {
  assert.deepEqual(
    normalizeDesktopToolMenuRequest({
      canOpenFlow: true,
      canOpenSideTask: true,
      x: 14.4,
      y: 28.6,
    }),
    { canOpenFlow: true, canOpenSideTask: true, x: 14, y: 29 },
  );
  assert.throws(
    () => normalizeDesktopToolMenuRequest({ x: 1 }),
    /provided together/,
  );
  assert.throws(
    () => normalizeDesktopToolMenuRequest({ x: Number.NaN, y: 1 }),
    /finite number/,
  );
});

test("builds only allowed tool actions and reports the selected action", () => {
  const selected = [];
  const template = createDesktopToolMenuTemplate(
    { canOpenFlow: false, canOpenSideTask: false },
    (action) => selected.push(action),
  );
  const labels = template
    .filter((item) => item.label)
    .map((item) => item.label);

  assert.deepEqual(labels, ["终端", "浏览器", "电脑", "文件", "侧边任务"]);
  assert.equal(template.at(-1).enabled, false);
  template.find((item) => item.label === "浏览器").click();
  assert.deepEqual(selected, ["browser"]);
  assert.deepEqual(TOOL_MENU_ACTIONS, [
    "flow",
    "terminal",
    "browser",
    "computer",
    "files",
    "side-task",
  ]);
});
