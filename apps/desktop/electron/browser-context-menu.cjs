const MAX_SELECTION_LABEL_LENGTH = 24;

function httpUrl(value) {
  if (typeof value !== "string" || value.length === 0) return null;
  if (Buffer.byteLength(value, "utf8") > 8192) return null;
  try {
    const parsed = new URL(value);
    if (
      !["http:", "https:"].includes(parsed.protocol) ||
      parsed.username ||
      parsed.password
    ) {
      return null;
    }
    return parsed.toString();
  } catch {
    return null;
  }
}

function selectionLabel(value) {
  const normalized = String(value || "")
    .trim()
    .replace(/\s+/g, " ");
  return normalized.length > MAX_SELECTION_LABEL_LENGTH
    ? `${normalized.slice(0, MAX_SELECTION_LABEL_LENGTH)}…`
    : normalized;
}

function createBrowserContextMenuTemplate(parameters, actions = {}) {
  const params = parameters && typeof parameters === "object" ? parameters : {};
  const groups = [];
  const linkUrl = httpUrl(params.linkURL);
  const sourceUrl = httpUrl(params.srcURL);
  const selection = String(params.selectionText || "").trim();
  const mediaType = String(params.mediaType || "none");
  const editFlags = params.editFlags || {};
  const editable = params.isEditable === true;
  const specificContent = Boolean(
    linkUrl || sourceUrl || selection || editable || mediaType !== "none",
  );

  if (linkUrl) {
    groups.push(
      compact([
        actionItem(
          "open-link-new-tab",
          "在新标签页中打开链接",
          () => actions.openNewTab?.(linkUrl),
          actions.openNewTab,
        ),
        actionItem(
          "copy-link-address",
          "复制链接地址",
          () => actions.copyText?.(linkUrl),
          actions.copyText,
        ),
        actionItem(
          "save-link-as",
          "链接另存为…",
          () =>
            actions.saveResource?.({
              kind: "link",
              suggestedFilename: params.suggestedFilename,
              url: linkUrl,
            }),
          actions.saveResource,
        ),
      ]),
    );
  }

  const canCopyImage =
    ["image", "canvas"].includes(mediaType) &&
    params.hasImageContents !== false;
  if (
    canCopyImage ||
    (sourceUrl && ["image", "audio", "video"].includes(mediaType))
  ) {
    const mediaLabel =
      mediaType === "image" ? "图片" : mediaType === "video" ? "视频" : "音频";
    groups.push(
      compact([
        sourceUrl
          ? actionItem(
              `open-${mediaType}-new-tab`,
              `在新标签页中打开${mediaLabel}`,
              () => actions.openNewTab?.(sourceUrl),
              actions.openNewTab,
            )
          : null,
        canCopyImage
          ? actionItem(
              "copy-image",
              "复制图片",
              () => actions.copyImage?.({ x: params.x, y: params.y }),
              actions.copyImage,
            )
          : null,
        sourceUrl
          ? actionItem(
              `copy-${mediaType}-address`,
              `复制${mediaLabel}地址`,
              () => actions.copyText?.(sourceUrl),
              actions.copyText,
            )
          : null,
        sourceUrl
          ? actionItem(
              `save-${mediaType}-as`,
              `${mediaLabel}另存为…`,
              () =>
                actions.saveResource?.({
                  kind: mediaType,
                  suggestedFilename: params.suggestedFilename,
                  url: sourceUrl,
                }),
              actions.saveResource,
            )
          : null,
      ]),
    );
  }

  if (editable) {
    groups.push(
      compact([
        editItem("undo", "撤销", editFlags.canUndo, actions.edit),
        editItem("redo", "重做", editFlags.canRedo, actions.edit),
      ]),
      compact([
        editItem("cut", "剪切", editFlags.canCut, actions.edit),
        editItem("copy", "复制", editFlags.canCopy, actions.edit),
        editItem("paste", "粘贴", editFlags.canPaste, actions.edit),
        editItem(
          "pasteAndMatchStyle",
          "粘贴为纯文本",
          editFlags.canPaste,
          actions.edit,
        ),
        editItem("delete", "删除", editFlags.canDelete, actions.edit),
      ]),
      compact([
        editItem("selectAll", "全选", editFlags.canSelectAll, actions.edit),
      ]),
    );
  } else if (selection) {
    groups.push(
      compact([
        actionItem(
          "copy-selection",
          "复制",
          () => actions.copyText?.(selection),
          actions.copyText,
        ),
        actionItem(
          "search-selection",
          `使用 Google 搜索“${selectionLabel(selection)}”`,
          () => actions.searchSelection?.(selection),
          actions.searchSelection,
        ),
      ]),
    );
  }

  if (!specificContent) {
    groups.push(
      compact([
        actionItem(
          "back",
          "后退",
          actions.back,
          actions.back,
          actions.canGoBack,
        ),
        actionItem(
          "forward",
          "前进",
          actions.forward,
          actions.forward,
          actions.canGoForward,
        ),
        actionItem("reload", "重新加载", actions.reload, actions.reload),
      ]),
    );
  }

  groups.push(
    compact([
      actionItem(
        "inspect-element",
        "检查",
        () => actions.inspect?.({ x: params.x, y: params.y }),
        actions.inspect,
      ),
    ]),
  );

  return joinGroups(groups);
}

function actionItem(id, label, click, action, enabled = true) {
  if (typeof action !== "function") return null;
  return { id, label, enabled: enabled !== false, click };
}

function editItem(command, label, enabled, edit) {
  return actionItem(
    `edit-${command}`,
    label,
    () => edit?.(command),
    edit,
    enabled,
  );
}

function compact(items) {
  return items.filter(Boolean);
}

function joinGroups(groups) {
  const template = [];
  for (const group of groups) {
    if (!group.length) continue;
    if (template.length) template.push({ type: "separator" });
    template.push(...group);
  }
  return template;
}

function popupBrowserContextMenu(Menu, window, parameters, actions) {
  if (!Menu || typeof Menu.buildFromTemplate !== "function") return false;
  const template = createBrowserContextMenuTemplate(parameters, actions);
  if (!template.length) return false;
  Menu.buildFromTemplate(template).popup({ window });
  return true;
}

module.exports = {
  createBrowserContextMenuTemplate,
  popupBrowserContextMenu,
};
