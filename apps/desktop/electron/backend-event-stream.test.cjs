const assert = require("node:assert/strict");
const { EventEmitter } = require("node:events");
const test = require("node:test");

const {
  backendEventStreamChannel,
  createBackendEventStreamManager,
  normalizeStreamPath,
} = require("./backend-event-stream.cjs");

class FakeSender extends EventEmitter {
  constructor(id = 1) {
    super();
    this.id = id;
    this.messages = [];
    this.destroyed = false;
  }

  isDestroyed() {
    return this.destroyed;
  }

  send(channel, message) {
    this.messages.push({ channel, message });
  }
}

test("accepts only local backend SSE endpoints", () => {
  const backendUrl = "http://127.0.0.1:8787";
  assert.equal(
    normalizeStreamPath(
      "/api/threads/00000000-0000-4000-8000-000000000001/events/stream?since=7&view=conversation",
      backendUrl,
    ).path,
    "/api/threads/00000000-0000-4000-8000-000000000001/events/stream?since=7&view=conversation",
  );
  assert.throws(
    () => normalizeStreamPath("https://example.com/events", backendUrl),
    /path is invalid/,
  );
  assert.throws(
    () => normalizeStreamPath("/api/settings", backendUrl),
    /endpoint is not allowed/,
  );
  assert.throws(
    () =>
      normalizeStreamPath(
        "/api/activity/events/stream?redirect=https://example.com",
        backendUrl,
      ),
    /query parameter is not allowed/,
  );
});

test("streams authenticated backend bytes through the main-process bridge", async () => {
  let observedAuthorization = null;
  const manager = createBackendEventStreamManager({
    getBackendUrl: () => "http://127.0.0.1:8787",
    getApiToken: () => "secret-token",
    fetchImpl: async (_url, init) => {
      observedAuthorization = init.headers.authorization;
      const encoder = new TextEncoder();
      return new Response(
        new ReadableStream({
          start(controller) {
            controller.enqueue(encoder.encode("data: first\n\n"));
            controller.enqueue(encoder.encode("data: second\n\n"));
            controller.close();
          },
        }),
        { status: 200 },
      );
    },
  });
  const sender = new FakeSender();

  await manager.open(sender, {
    streamId: "stream-1",
    path: "/api/activity/events/stream",
  });

  assert.equal(observedAuthorization, "Bearer secret-token");
  assert.equal(manager.activeCount(), 0);
  assert.deepEqual(
    sender.messages.map(({ channel, message }) => [
      channel,
      message.type,
      message.chunk,
    ]),
    [
      [backendEventStreamChannel, "connected", undefined],
      [backendEventStreamChannel, "chunk", "data: first\n\n"],
      [backendEventStreamChannel, "chunk", "data: second\n\n"],
      [backendEventStreamChannel, "closed", undefined],
    ],
  );
});

test("closing a bridge aborts its main-process request", async () => {
  let aborted = false;
  const manager = createBackendEventStreamManager({
    getBackendUrl: () => "http://127.0.0.1:8787",
    getApiToken: () => "token",
    fetchImpl: async (_url, init) => {
      await new Promise((resolve) => {
        init.signal.addEventListener(
          "abort",
          () => {
            aborted = true;
            resolve();
          },
          { once: true },
        );
      });
      throw new DOMException("aborted", "AbortError");
    },
  });
  const sender = new FakeSender();
  const completion = manager.open(sender, {
    streamId: "stream-2",
    path: "/api/activity/events/stream",
  });

  assert.equal(manager.close(sender, "stream-2"), true);
  await completion;

  assert.equal(aborted, true);
  assert.equal(manager.activeCount(), 0);
  assert.equal(
    sender.messages.some(({ message }) => message.type === "error"),
    false,
  );
});
