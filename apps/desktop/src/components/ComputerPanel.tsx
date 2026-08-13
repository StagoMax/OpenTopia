import { useCallback, useEffect, useMemo, useState } from "react";
import { Camera, Monitor, RefreshCw, Square } from "lucide-react";
import type { ApiClient } from "../api/client";
import type {
  AgentEvent,
  ComputerObservation,
  ComputerScreenshot,
  ComputerWindowTarget,
  ModelContentPart,
} from "../types";

export function ComputerPanel({
  client,
  threadId,
  events,
}: {
  client: ApiClient | null;
  threadId: string | null;
  events?: AgentEvent[];
}) {
  const [windows, setWindows] = useState<ComputerWindowTarget[]>([]);
  const [selectedWindowId, setSelectedWindowId] = useState("");
  const [observation, setObservation] = useState<ComputerObservation | null>(
    null,
  );
  const [loading, setLoading] = useState(false);
  const [observing, setObserving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const selectedWindow = useMemo(
    () =>
      windows.find((window) => window.windowId === selectedWindowId) ?? null,
    [selectedWindowId, windows],
  );
  const latestAgentObservation = useMemo<{
    screenshot: ComputerScreenshot;
    capturedAt: string;
  } | null>(() => {
    for (const event of [...(events ?? [])].reverse()) {
      if (event.payload.type !== "tool_call_finished") continue;
      const result = event.payload.result;
      if (!isComputerResult(result.metadata)) continue;
      const image = result.content?.find(
        (part): part is Extract<ModelContentPart, { type: "image" }> =>
          part.type === "image",
      );
      if (image) {
        return {
          screenshot: { mimeType: image.content_type, bytes: image.data },
          capturedAt: event.createdAt,
        };
      }
    }
    return null;
  }, [events]);
  const visibleObservation = useMemo(() => {
    if (
      observation?.screenshot &&
      (!latestAgentObservation ||
        Date.parse(observation.capturedAt) >=
          Date.parse(latestAgentObservation.capturedAt))
    ) {
      return {
        screenshot: observation.screenshot,
        title: observation.target.title,
        detail: observation.target.executable ?? observation.target.title,
        observationId: observation.observationId,
      };
    }
    if (latestAgentObservation) {
      return {
        screenshot: latestAgentObservation.screenshot,
        title: "Agent observation",
        detail: "Screenshot returned by the computer tool",
        observationId: "latest",
      };
    }
    return null;
  }, [latestAgentObservation, observation]);

  const refresh = useCallback(async () => {
    if (!client || !threadId) return;
    setLoading(true);
    setError(null);
    try {
      const next = await client.listComputerWindows(threadId);
      setWindows(next);
      setSelectedWindowId((current) =>
        next.some((window) => window.windowId === current)
          ? current
          : (next[0]?.windowId ?? ""),
      );
    } catch (cause) {
      setWindows([]);
      setSelectedWindowId("");
      setError(errorMessage(cause));
    } finally {
      setLoading(false);
    }
  }, [client, threadId]);

  useEffect(() => {
    setObservation(null);
    setError(null);
    setWindows([]);
    setSelectedWindowId("");
    if (client && threadId) void refresh();
  }, [client, refresh, threadId]);

  async function observe() {
    if (!client || !threadId || !selectedWindowId || observing) return;
    setObserving(true);
    setError(null);
    try {
      setObservation(
        await client.observeComputerWindow(threadId, selectedWindowId),
      );
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setObserving(false);
    }
  }

  async function close() {
    if (!client || !threadId) return;
    setError(null);
    try {
      await client.closeComputerSession(threadId);
      setObservation(null);
    } catch (cause) {
      setError(errorMessage(cause));
    }
  }

  return (
    <section className="computer-panel" aria-label="Computer observation">
      <div className="computer-toolbar">
        <Monitor aria-hidden="true" size={15} />
        <select
          aria-label="Desktop window to observe"
          disabled={!threadId || loading || windows.length === 0}
          value={selectedWindowId}
          onChange={(event) => setSelectedWindowId(event.target.value)}
        >
          {windows.length === 0 ? (
            <option value="">No available windows</option>
          ) : (
            windows.map((window) => (
              <option key={window.windowId} value={window.windowId}>
                {window.title}
              </option>
            ))
          )}
        </select>
        <button
          aria-label="Refresh desktop windows"
          className="icon-button small"
          disabled={!threadId || loading}
          title="Refresh desktop windows"
          type="button"
          onClick={() => void refresh()}
        >
          <RefreshCw className={loading ? "spin" : undefined} size={14} />
        </button>
        <button
          aria-label="Observe selected window"
          className="icon-button small"
          disabled={!selectedWindowId || observing}
          title="Observe selected window"
          type="button"
          onClick={() => void observe()}
        >
          <Camera className={observing ? "spin" : undefined} size={14} />
        </button>
        <button
          aria-label="Close computer session"
          className="icon-button small danger"
          disabled={!threadId}
          title="Close computer session"
          type="button"
          onClick={() => void close()}
        >
          <Square fill="currentColor" size={12} />
        </button>
      </div>

      {error ? (
        <div className="computer-status" role="alert">
          {error}
        </div>
      ) : visibleObservation ? (
        <div className="computer-observation">
          <img
            alt={`Latest computer observation of ${visibleObservation.title}`}
            src={imageUrl(
              visibleObservation.screenshot.mimeType,
              visibleObservation.screenshot.bytes,
            )}
          />
          <footer>
            <span title={visibleObservation.detail}>
              {visibleObservation.title}
            </span>
            <code>{visibleObservation.observationId}</code>
          </footer>
        </div>
      ) : (
        <div className="computer-status">
          {selectedWindow
            ? selectedWindow.title
            : "Select a window to start an observation."}
        </div>
      )}
    </section>
  );
}

function isComputerResult(metadata: unknown): boolean {
  if (!metadata || typeof metadata !== "object") return false;
  const value = metadata as { toolName?: unknown; computer?: unknown };
  return value.toolName === "computer" && Boolean(value.computer);
}

function imageUrl(mimeType: string, bytes: number[]): string {
  let binary = "";
  const chunkSize = 0x8000;
  for (let index = 0; index < bytes.length; index += chunkSize) {
    binary += String.fromCharCode(...bytes.slice(index, index + chunkSize));
  }
  return `data:${mimeType};base64,${btoa(binary)}`;
}

function errorMessage(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}
