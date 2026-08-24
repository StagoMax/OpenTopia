const TOOL_MENU_ACTIONS = Object.freeze([
  "flow",
  "terminal",
  "browser",
  "computer",
  "files",
  "side-task",
]);

function normalizeCoordinate(value, name) {
  if (value === undefined) return undefined;
  if (!Number.isFinite(value)) {
    throw new TypeError(`Desktop tool menu ${name} must be a finite number.`);
  }
  return Math.max(0, Math.round(value));
}

function normalizeDesktopToolMenuRequest(request) {
  if (!request || typeof request !== "object" || Array.isArray(request)) {
    throw new TypeError("Desktop tool menu request must be an object.");
  }

  const x = normalizeCoordinate(request.x, "x");
  const y = normalizeCoordinate(request.y, "y");
  if ((x === undefined) !== (y === undefined)) {
    throw new TypeError("Desktop tool menu x and y must be provided together.");
  }

  return {
    canOpenFlow: request.canOpenFlow === true,
    canOpenSideTask: request.canOpenSideTask === true,
    x,
    y,
  };
}

function createDesktopToolMenuTemplate(options, select) {
  const choose = (action) => () => select(action);
  return [
    ...(options.canOpenFlow ? [{ label: "Flow", click: choose("flow") }] : []),
    { label: "终端", click: choose("terminal") },
    { label: "浏览器", click: choose("browser") },
    { label: "电脑", click: choose("computer") },
    { label: "文件", click: choose("files") },
    { type: "separator" },
    {
      label: "侧边任务",
      enabled: options.canOpenSideTask,
      click: choose("side-task"),
    },
  ];
}

module.exports = {
  TOOL_MENU_ACTIONS,
  createDesktopToolMenuTemplate,
  normalizeDesktopToolMenuRequest,
};
