const backendEventStreamChannel = "backend-event-stream:message";
const maxStreamPathLength = 2_048;
const maxStreamIdLength = 128;

function createBackendEventStreamManager({
  getBackendUrl,
  getApiToken,
  fetchImpl = globalThis.fetch,
  logger = () => {},
}) {
  if (typeof getBackendUrl !== "function") {
    throw new TypeError("getBackendUrl must be a function");
  }
  if (typeof getApiToken !== "function") {
    throw new TypeError("getApiToken must be a function");
  }
  if (typeof fetchImpl !== "function") {
    throw new TypeError("fetchImpl must be a function");
  }

  const streams = new Map();
  const senderStreams = new Map();

  function open(sender, request) {
    const streamId = normalizeStreamId(request?.streamId);
    const { path, url } = normalizeStreamPath(
      request?.path,
      getBackendUrl(),
    );
    const key = streamKey(sender, streamId);
    closeEntry(streams.get(key), "replaced");

    const controller = new AbortController();
    const entry = {
      key,
      sender,
      streamId,
      path,
      controller,
      startedAt: Date.now(),
    };
    streams.set(key, entry);
    retainSenderEntry(entry);
    logger("info", "backend.event-stream.opening", {
      streamId,
      path,
      senderId: sender.id,
    });

    const completion = pump(entry, url);
    entry.completion = completion;
    return completion;
  }

  function reject(sender, request, error) {
    const streamId = safeStreamId(request?.streamId);
    if (!streamId) return false;
    send(sender, {
      streamId,
      type: "error",
      error: error instanceof Error ? error.message : String(error),
    });
    logger("warn", "backend.event-stream.rejected", {
      streamId,
      senderId: sender?.id,
      error: error instanceof Error ? error.message : String(error),
    });
    return true;
  }

  function close(sender, rawStreamId) {
    const streamId = safeStreamId(rawStreamId);
    if (!streamId) return false;
    return closeEntry(streams.get(streamKey(sender, streamId)), "renderer");
  }

  function closeSender(sender) {
    const retained = senderStreams.get(sender.id);
    if (!retained) return;
    for (const key of [...retained.keys]) {
      closeEntry(streams.get(key), "sender-destroyed");
    }
    senderStreams.delete(sender.id);
  }

  function closeAll() {
    for (const entry of [...streams.values()]) {
      closeEntry(entry, "manager-closed");
    }
  }

  async function pump(entry, url) {
    try {
      const response = await fetchImpl(url, {
        headers: {
          authorization: `Bearer ${getApiToken()}`,
          accept: "text/event-stream",
        },
        signal: entry.controller.signal,
      });
      if (!response.ok) {
        throw new Error(
          `Event stream failed: ${response.status} ${response.statusText}`,
        );
      }
      if (!response.body) {
        throw new Error("Event stream response has no body");
      }

      sendEntry(entry, { type: "connected", status: response.status });
      logger("info", "backend.event-stream.connected", {
        streamId: entry.streamId,
        path: entry.path,
        senderId: entry.sender.id,
        elapsedMs: Date.now() - entry.startedAt,
      });

      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      try {
        while (!entry.controller.signal.aborted) {
          const { done, value } = await reader.read();
          if (done) break;
          const chunk = decoder.decode(value, { stream: true });
          if (chunk) sendEntry(entry, { type: "chunk", chunk });
        }
        const tail = decoder.decode();
        if (tail && !entry.controller.signal.aborted) {
          sendEntry(entry, { type: "chunk", chunk: tail });
        }
      } finally {
        reader.releaseLock();
      }

      if (!entry.controller.signal.aborted) {
        sendEntry(entry, { type: "closed", reason: "eof" });
      }
    } catch (error) {
      if (!entry.controller.signal.aborted) {
        sendEntry(entry, {
          type: "error",
          error: error instanceof Error ? error.message : String(error),
        });
        logger("warn", "backend.event-stream.failed", {
          streamId: entry.streamId,
          path: entry.path,
          senderId: entry.sender.id,
          elapsedMs: Date.now() - entry.startedAt,
          error: error instanceof Error ? error.message : String(error),
        });
      }
    } finally {
      releaseEntry(entry);
    }
  }

  function retainSenderEntry(entry) {
    let retained = senderStreams.get(entry.sender.id);
    if (!retained) {
      const destroyed = () => closeSender(entry.sender);
      retained = { sender: entry.sender, keys: new Set(), destroyed };
      senderStreams.set(entry.sender.id, retained);
      entry.sender.once?.("destroyed", destroyed);
    }
    retained.keys.add(entry.key);
  }

  function releaseEntry(entry) {
    if (streams.get(entry.key) !== entry) return;
    streams.delete(entry.key);
    const retained = senderStreams.get(entry.sender.id);
    retained?.keys.delete(entry.key);
    if (retained && retained.keys.size === 0) {
      retained.sender.removeListener?.("destroyed", retained.destroyed);
      senderStreams.delete(entry.sender.id);
    }
    logger("info", "backend.event-stream.closed", {
      streamId: entry.streamId,
      path: entry.path,
      senderId: entry.sender.id,
      elapsedMs: Date.now() - entry.startedAt,
    });
  }

  function closeEntry(entry, reason) {
    if (!entry) return false;
    entry.controller.abort(reason);
    sendEntry(entry, { type: "closed", reason });
    releaseEntry(entry);
    return true;
  }

  function sendEntry(entry, message) {
    if (streams.get(entry.key) !== entry) return;
    send(entry.sender, { streamId: entry.streamId, ...message });
  }

  return {
    open,
    reject,
    close,
    closeSender,
    closeAll,
    activeCount: () => streams.size,
  };
}

function normalizeStreamPath(rawPath, backendUrl) {
  if (
    typeof rawPath !== "string" ||
    !rawPath.startsWith("/") ||
    rawPath.length > maxStreamPathLength
  ) {
    throw new TypeError("Backend event stream path is invalid");
  }
  const base = new URL(backendUrl);
  const url = new URL(rawPath, base);
  if (url.origin !== base.origin || url.hash) {
    throw new Error("Backend event stream must use the configured backend");
  }

  const allowedPath =
    url.pathname === "/api/activity/events/stream" ||
    /^\/api\/threads\/[a-z0-9-]{1,128}\/(?:events\/stream|agents\/events\/stream|terminal\/stream)$/i.test(
      url.pathname,
    );
  if (!allowedPath) {
    throw new Error("Backend event stream endpoint is not allowed");
  }
  for (const [key, value] of url.searchParams) {
    if (key === "since" && /^\d+$/.test(value)) continue;
    if (key === "view" && value === "conversation") continue;
    throw new Error(`Backend event stream query parameter is not allowed: ${key}`);
  }
  return { path: `${url.pathname}${url.search}`, url };
}

function normalizeStreamId(rawStreamId) {
  const streamId = safeStreamId(rawStreamId);
  if (!streamId) throw new TypeError("Backend event stream id is invalid");
  return streamId;
}

function safeStreamId(rawStreamId) {
  return typeof rawStreamId === "string" &&
    rawStreamId.length > 0 &&
    rawStreamId.length <= maxStreamIdLength &&
    /^[a-z0-9-]+$/i.test(rawStreamId)
    ? rawStreamId
    : null;
}

function streamKey(sender, streamId) {
  return `${sender.id}:${streamId}`;
}

function send(sender, message) {
  if (!sender || sender.isDestroyed?.()) return;
  sender.send(backendEventStreamChannel, message);
}

module.exports = {
  backendEventStreamChannel,
  createBackendEventStreamManager,
  normalizeStreamPath,
};
