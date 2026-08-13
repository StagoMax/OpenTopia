import assert from "node:assert/strict";
import test from "node:test";

import type { AgentEvent, ContextSourceRef, Message } from "./types";
import { attachmentsByAssistantMessage } from "./conversationAttachmentReferences.ts";

const source: ContextSourceRef = {
  id: "attachment-1",
  path: "C:/Temp/report.pdf",
  name: "report.pdf",
  kind: "document",
  contentType: "application/pdf",
  bytes: 42,
  truncated: false,
};

function message(
  id: string,
  role: Message["role"],
  parts: Message["parts"],
): Message {
  return {
    id,
    threadId: "thread-1",
    role,
    parts,
    createdAt: "2026-08-13T00:00:00.000Z",
  };
}

function event(
  id: string,
  turnId: string,
  payload: AgentEvent["payload"],
): AgentEvent {
  return {
    id,
    seq: Number.parseInt(id.replace(/\D/g, ""), 10) || 1,
    threadId: "thread-1",
    turnId,
    payload,
    createdAt: "2026-08-13T00:00:00.000Z",
  };
}

test("associates an assistant message with sources from its own turn", () => {
  const user = message("user-1", "user", [{ type: "source_ref", source }]);
  const assistant = message("assistant-1", "assistant", [
    { type: "text", text: "report.pdf" },
  ]);
  const events = [
    event("event-1", "turn-1", {
      type: "turn_started",
      user_message_id: user.id,
    }),
    event("event-2", "turn-1", {
      type: "assistant_message",
      message: assistant,
    }),
  ];

  assert.deepEqual(
    attachmentsByAssistantMessage([user, assistant], events).get(assistant.id),
    [source],
  );
});

test("does not leak attachments past the next user message", () => {
  const messages = [
    message("user-1", "user", [{ type: "source_ref", source }]),
    message("assistant-1", "assistant", [{ type: "text", text: "report.pdf" }]),
    message("user-2", "user", [{ type: "text", text: "next" }]),
    message("assistant-2", "assistant", [{ type: "text", text: "report.pdf" }]),
  ];

  const links = attachmentsByAssistantMessage(messages, []);
  assert.deepEqual(links.get("assistant-1"), [source]);
  assert.deepEqual(links.get("assistant-2"), []);
});

test("falls back to message order when a lifecycle event lost its user message", () => {
  const user = message("user-1", "user", [{ type: "source_ref", source }]);
  const assistant = message("assistant-1", "assistant", [
    { type: "text", text: "report.pdf" },
  ]);
  const events = [
    event("event-1", "turn-1", {
      type: "turn_started",
      user_message_id: "missing-user",
    }),
    event("event-2", "turn-1", {
      type: "assistant_message",
      message: assistant,
    }),
  ];

  assert.deepEqual(
    attachmentsByAssistantMessage([user, assistant], events).get(assistant.id),
    [source],
  );
});
