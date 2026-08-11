import assert from "node:assert/strict";
import test from "node:test";

import type * as ConversationMessageMetaModule from "./conversationMessageMeta";
import type { MessagePart } from "./types";

const conversationMessageMeta: typeof ConversationMessageMetaModule =
  await import("./conversationMessageMeta" + ".ts");

const { conversationMessageCopyText, formatConversationMessageTimestamp } =
  conversationMessageMeta;

test("builds copy text from the visible textual message parts", () => {
  const parts: MessagePart[] = [
    { type: "text", text: "第一段" },
    { type: "image_ref", image_id: "image-1" },
    { type: "file_ref", path: "src/App.tsx" },
    {
      type: "source_ref",
      source: {
        id: "source-1",
        path: "docs/notes.md",
        name: "notes.md",
        kind: "text",
        contentType: "text/markdown",
        bytes: 12,
        truncated: false,
      },
    },
  ];

  assert.equal(
    conversationMessageCopyText(parts),
    "第一段\n\nsrc/App.tsx\n\ndocs/notes.md",
  );
});

test("keeps message indentation while removing blank outer lines", () => {
  assert.equal(
    conversationMessageCopyText([{ type: "text", text: "\n    code\n\n" }]),
    "    code",
  );
});

test("formats a local message timestamp with weekday and minute", () => {
  const localDate = new Date(2026, 7, 10, 18, 37, 12);
  const timestamp = formatConversationMessageTimestamp(localDate.toISOString());

  assert.equal(timestamp?.label, "星期一 · 18:37");
  assert.match(timestamp?.title ?? "", /2026/);
  assert.equal(formatConversationMessageTimestamp("invalid"), null);
});
