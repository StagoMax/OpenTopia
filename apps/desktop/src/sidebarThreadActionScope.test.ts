import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const sidebarThreadRowSource = readFileSync(
  new URL("./features/sidebar/SidebarThreadRow.tsx", import.meta.url),
  "utf8",
);

test("keeps project lifecycle actions out of task-row menus", () => {
  assert.doesNotMatch(
    sidebarThreadRowSource,
    /onRemoveProject|onToggleProjectPinned|从最近项目移除|固定项目/,
  );
  assert.match(sidebarThreadRowSource, /onArchive\(thread: Thread\): void/);
  assert.match(sidebarThreadRowSource, /<span>归档任务<\/span>/);
});
