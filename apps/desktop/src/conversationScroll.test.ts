import assert from "node:assert/strict";
import test from "node:test";

import type * as ConversationScrollModule from "./conversationScroll";

const conversationScroll: typeof ConversationScrollModule = await import(
  "./conversationScroll" + ".ts"
);

const { conversationScrollBottomThreshold, isConversationScrollNearEnd } =
  conversationScroll;

test("keeps a conversation pinned when it is at or near the end", () => {
  assert.equal(
    isConversationScrollNearEnd({
      scrollHeight: 1_000,
      clientHeight: 400,
      scrollTop: 600,
    }),
    true,
  );
  assert.equal(
    isConversationScrollNearEnd({
      scrollHeight: 1_000,
      clientHeight: 400,
      scrollTop: 600 - conversationScrollBottomThreshold,
    }),
    true,
  );
});

test("releases the conversation when the user scrolls above the threshold", () => {
  assert.equal(
    isConversationScrollNearEnd({
      scrollHeight: 1_000,
      clientHeight: 400,
      scrollTop: 600 - conversationScrollBottomThreshold - 1,
    }),
    false,
  );
});

test("treats content shorter than the viewport as pinned", () => {
  assert.equal(
    isConversationScrollNearEnd({
      scrollHeight: 320,
      clientHeight: 400,
      scrollTop: 0,
    }),
    true,
  );
});
