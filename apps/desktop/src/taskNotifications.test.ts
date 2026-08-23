import assert from "node:assert/strict";
import test from "node:test";

import type * as TaskNotificationsModule from "./taskNotifications";
import type { AgentEvent, Message } from "./types";

const {
  formatTaskCompletionNotificationBody,
  formatTaskCompletionNotificationTitle,
  resolveTaskCompletionNotificationContent,
  taskNotificationReplyMaxChars,
  truncateNotificationText,
} = (await import(
  "./taskNotifications" + ".ts"
)) as typeof TaskNotificationsModule;

function message(
  id: string,
  role: Message["role"],
  text: string,
): Message {
  return {
    id,
    threadId: "thread-1",
    role,
    parts: [{ type: "text", text }],
    createdAt: "2026-08-23T00:00:00Z",
  };
}

function event(
  id: string,
  seq: number,
  payload: AgentEvent["payload"],
  turnId: string,
): AgentEvent {
  return {
    id,
    seq,
    threadId: "thread-1",
    turnId,
    createdAt: `2026-08-23T00:00:0${seq}Z`,
    payload,
  };
}

test("uses the completed turn's user message and final assistant reply", () => {
  const firstUser = message("user-1", "user", "之前的问题");
  const lastUser = message("user-2", "user", "最后发送的问题");
  const finalReply = message("assistant-2", "assistant", "这是最终回复。");
  const finished = event(
    "finished-2",
    5,
    { type: "turn_finished", summary: "Provider agent turn completed." },
    "turn-2",
  );
  const content = resolveTaskCompletionNotificationContent(
    [firstUser, lastUser],
    [
      event(
        "started-1",
        1,
        { type: "turn_started", user_message_id: firstUser.id },
        "turn-1",
      ),
      event(
        "started-2",
        2,
        { type: "turn_started", user_message_id: lastUser.id },
        "turn-2",
      ),
      event(
        "assistant-2",
        4,
        { type: "assistant_message", message: finalReply },
        "turn-2",
      ),
    ],
    finished,
  );

  assert.deepEqual(content, {
    userMessage: "最后发送的问题",
    reply: "这是最终回复。",
  });
  assert.equal(
    formatTaskCompletionNotificationTitle(content),
    "最后发送的问题",
  );
  assert.equal(formatTaskCompletionNotificationBody(content), "这是最终回复。");
});

test("falls back to the completion summary when no assistant message exists", () => {
  const user = message("user-1", "user", "运行检查");
  const oldReply = {
    ...message("assistant-old", "assistant", "旧回复，不应显示。"),
    createdAt: "2026-08-22T23:59:00Z",
  };
  const finished = event(
    "finished",
    2,
    { type: "turn_finished", summary: "检查已完成。" },
    "turn-1",
  );
  const content = resolveTaskCompletionNotificationContent(
    [oldReply, user],
    [
      event(
        "started",
        1,
        { type: "turn_started", user_message_id: user.id },
        "turn-1",
      ),
    ],
    finished,
  );

  assert.deepEqual(content, {
    userMessage: "运行检查",
    reply: "检查已完成。",
  });
});

test("shortens an oversized Unicode reply with an ellipsis", () => {
  const body = formatTaskCompletionNotificationBody({
    userMessage: "说明结果",
    reply: "界".repeat(taskNotificationReplyMaxChars + 20),
  });
  assert.equal(Array.from(body).length, taskNotificationReplyMaxChars);
  assert.equal(body, `${"界".repeat(taskNotificationReplyMaxChars - 1)}…`);
});

test("keeps an emoji-heavy reply within its card allowance", () => {
  const body = formatTaskCompletionNotificationBody({
    userMessage: "说明结果",
    reply: "😀".repeat(taskNotificationReplyMaxChars),
  });

  assert.ok(body.length <= taskNotificationReplyMaxChars);
  assert.equal(body.endsWith("…"), true);
});

test("does not split surrogate pairs when shortening notification text", () => {
  assert.equal(truncateNotificationText("😀😀😀", 2), "😀…");
});

test("keeps an emoji-heavy title within the main-process limit", () => {
  const title = formatTaskCompletionNotificationTitle({
    userMessage: "😀".repeat(100),
    reply: "完成",
  });

  assert.ok(title.length <= 120);
  assert.equal(title.endsWith("…"), true);
});

test("uses a current assistant message even when its event has no turn id", () => {
  const user = message("user-1", "user", "检查结果");
  const assistant = {
    ...message("assistant-1", "assistant", "已检查完成。"),
    createdAt: "2026-08-23T00:00:03Z",
  };
  const finished = event(
    "finished",
    4,
    { type: "turn_finished", summary: "Provider agent turn completed." },
    "turn-1",
  );
  const content = resolveTaskCompletionNotificationContent(
    [user],
    [
      event(
        "started",
        1,
        { type: "turn_started", user_message_id: user.id },
        "turn-1",
      ),
      {
        ...event(
          "assistant",
          3,
          { type: "assistant_message", message: assistant },
          "turn-1",
        ),
        turnId: null,
      },
    ],
    finished,
  );

  assert.equal(content.reply, "已检查完成。");
});
