import assert from "node:assert/strict";
import test from "node:test";

import type { ApiClient } from "./api/client";
import type * as HistoryLoaderModule from "./conversationHistoryLoader";
import type { Message } from "./types";

const { ConversationHistoryLoader }: typeof HistoryLoaderModule = await import(
  "./conversationHistoryLoader" + ".ts"
);

function message(index: number): Message {
  return {
    id: `message-${String(index).padStart(3, "0")}`,
    threadId: "thread-1",
    role: "user",
    parts: [{ type: "text", text: String(index) }],
    createdAt: `2026-08-24T00:${String(index).padStart(2, "0")}:00Z`,
  };
}

test("initial history keeps only the newest bounded message page", async () => {
  const source = Array.from({ length: 61 }, (_, index) => message(index));
  const client = {
    listMessages: () => Promise.resolve(source),
  } as unknown as ApiClient;
  const loader = new ConversationHistoryLoader(client, "thread-1");

  const page = await loader.loadInitialMessages(new AbortController().signal);

  assert.equal(page.hasOlderMessages, true);
  assert.equal(page.messages.length, 60);
  assert.equal(page.messages[0].id, "message-001");
  assert.equal(page.messages.at(-1)?.id, "message-060");
});

test("return visits walk only the forward message delta cursor", async () => {
  const requestedAfter: Array<string | undefined> = [];
  const firstPage = Array.from({ length: 200 }, (_, index) =>
    message(index + 1),
  );
  const lastPage = [message(201)];
  const client = {
    listMessages: (
      _threadId: string,
      _signal: AbortSignal,
      page: { after?: Pick<Message, "id" | "createdAt"> },
    ) => {
      requestedAfter.push(page.after?.id);
      return Promise.resolve(
        requestedAfter.length === 1 ? firstPage : lastPage,
      );
    },
  } as unknown as ApiClient;
  const loader = new ConversationHistoryLoader(client, "thread-1");

  const page = await loader.loadMessageDelta(
    [message(0)],
    true,
    new AbortController().signal,
  );

  assert.deepEqual(requestedAfter, ["message-000", "message-200"]);
  assert.equal(page.messages.length, 201);
  assert.equal(page.hasOlderMessages, true);
});
