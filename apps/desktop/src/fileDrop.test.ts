import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { hasFileDragPayload } from "./fileDrop.ts";

const appSource = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
const composerSource = readFileSync(
  new URL("./features/composer/Composer.tsx", import.meta.url),
  "utf8",
);
const newTaskStateSource = readFileSync(
  new URL(
    "./features/conversation/ConversationEmptyStates.tsx",
    import.meta.url,
  ),
  "utf8",
);

test("recognizes operating-system file drags", () => {
  assert.equal(hasFileDragPayload(["text/plain", "Files"]), true);
});

test("leaves text and in-app drags alone", () => {
  assert.equal(hasFileDragPayload([]), false);
  assert.equal(hasFileDragPayload(["text/plain", "text/uri-list"]), false);
});

test("keeps file drops owned by the shared conversation boundary", () => {
  assert.doesNotMatch(
    composerSource,
    /onDragEnter=|onDragOver=|onDragLeave=|onDrop=|fileDropScope|composer-drop-target/,
  );
  assert.match(
    composerSource,
    /fileDropHandleRef: \{ current: ComposerFileDropHandle \| null \};/,
  );
  assert.match(
    appSource,
    /<NewTaskState[\s\S]{0,200}fileDropHandleRef=\{conversationComposerFileDropHandle\}/,
  );
  assert.match(
    newTaskStateSource,
    /<Composer[\s\S]{0,200}fileDropHandleRef=\{fileDropHandleRef\}/,
  );
});
