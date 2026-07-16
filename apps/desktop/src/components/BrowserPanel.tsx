import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import {
  Camera,
  Download,
  FolderOpen,
  Keyboard,
  Loader2,
  MousePointer2,
  RefreshCw,
  Square,
} from "lucide-react";
import { ApiClient } from "../api/client";
import { openPath } from "../platform";
import type {
  AgentEvent,
  BrowserContent,
  BrowserOutput,
  ModelContentPart,
  ToolResult,
} from "../types";

type BrowserAction =
  | "navigate"
  | "snapshot"
  | "screenshot"
  | "click"
  | "type"
  | "download"
  | "close";

export function BrowserPanel({
  client,
  threadId,
  events,
  pendingApprovalIds,
  decidingApprovalId,
  onDecideApproval,
}: {
  client: ApiClient | null;
  threadId: string | null;
  events: AgentEvent[];
  pendingApprovalIds: string[];
  decidingApprovalId: string | null;
  onDecideApproval(approvalId: string, approved: boolean): void;
}) {
  const [url, setUrl] = useState("");
  const [selector, setSelector] = useState("");
  const [text, setText] = useState("");
  const [output, setOutput] = useState<BrowserOutput | null>(null);
  const [isRunning, setIsRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const activeThreadIdRef = useRef(threadId);
  const requestVersionRef = useRef(0);
  const manualOperationRunningRef = useRef(false);
  const manualEventBarrierRef = useRef<{
    completedAt: number;
    seq: number;
  } | null>(null);
  const latestEventSeqRef = useRef(0);
  const handledBrowserEventIdRef = useRef<string | null>(null);

  activeThreadIdRef.current = threadId;

  const latestEventSeq = useMemo(
    () =>
      events.reduce(
        (latest, event) =>
          event.threadId === threadId ? Math.max(latest, event.seq) : latest,
        0,
      ),
    [events, threadId],
  );
  latestEventSeqRef.current = latestEventSeq;

  const latestBrowserEvent = useMemo(() => {
    let latest: AgentEvent | null = null;
    for (const event of events) {
      if (
        event.threadId !== threadId ||
        event.payload.type !== "tool_call_finished" ||
        !isBrowserToolResult(event.payload.result)
      ) {
        continue;
      }
      if (!latest || event.seq > latest.seq) latest = event;
    }
    return latest;
  }, [events, threadId]);

  useLayoutEffect(() => {
    requestVersionRef.current += 1;
    manualOperationRunningRef.current = false;
    manualEventBarrierRef.current = null;
    handledBrowserEventIdRef.current = null;
    setUrl("");
    setOutput(null);
    setError(null);
    setIsRunning(false);
  }, [threadId]);

  useEffect(() => {
    if (
      !latestBrowserEvent ||
      handledBrowserEventIdRef.current === latestBrowserEvent.id ||
      manualOperationRunningRef.current
    ) {
      return;
    }

    const barrier = manualEventBarrierRef.current;
    const eventTimestamp = Date.parse(latestBrowserEvent.createdAt);
    if (
      barrier &&
      (latestBrowserEvent.seq <= barrier.seq ||
        (Number.isFinite(eventTimestamp) &&
          eventTimestamp < barrier.completedAt))
    ) {
      handledBrowserEventIdRef.current = latestBrowserEvent.id;
      return;
    }

    if (latestBrowserEvent.payload.type !== "tool_call_finished") return;
    const result = latestBrowserEvent.payload.result;
    const next = browserOutputFromToolResult(result);
    handledBrowserEventIdRef.current = latestBrowserEvent.id;
    setOutput(next);
    if (next.url) setUrl(next.url);
    setError(browserToolError(result));
  }, [latestBrowserEvent]);

  const snapshotText = useMemo(
    () =>
      output?.contents.find(
        (content): content is Extract<BrowserContent, { type: "text" }> =>
          content.type === "text",
      )?.text ?? "",
    [output],
  );
  const screenshot = useMemo(
    () =>
      output?.contents.find(
        (content): content is Extract<BrowserContent, { type: "image" }> =>
          content.type === "image",
      ) ?? null,
    [output],
  );
  const downloads = useMemo(
    () => output?.contents.filter((content) => content.type === "file") ?? [],
    [output],
  );
  const pendingBrowserApproval = useMemo(
    () =>
      [...events]
        .reverse()
        .find(
          (event) =>
            event.payload.type === "approval_requested" &&
            event.payload.action.startsWith("browser:domain:") &&
            pendingApprovalIds.includes(event.payload.approval_id),
        )?.payload,
    [events, pendingApprovalIds],
  );

  async function run(action: BrowserAction) {
    if (!client || !threadId || isRunning) return;
    const requestVersion = ++requestVersionRef.current;
    const requestThreadId = threadId;
    manualOperationRunningRef.current = true;
    setIsRunning(true);
    setError(null);
    try {
      const next = await client.runBrowserCommand(threadId, {
        action,
        url: action === "navigate" || action === "download" ? url : undefined,
        selector:
          action === "click" || action === "type" ? selector : undefined,
        text: action === "type" ? text : undefined,
      });
      if (
        requestVersionRef.current !== requestVersion ||
        activeThreadIdRef.current !== requestThreadId
      ) {
        return;
      }
      setOutput(next);
      if (next.url) setUrl(next.url);
    } catch (cause) {
      if (
        requestVersionRef.current !== requestVersion ||
        activeThreadIdRef.current !== requestThreadId
      ) {
        return;
      }
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      if (
        requestVersionRef.current !== requestVersion ||
        activeThreadIdRef.current !== requestThreadId
      ) {
        return;
      }
      manualEventBarrierRef.current = {
        completedAt: Date.now(),
        seq: latestEventSeqRef.current,
      };
      manualOperationRunningRef.current = false;
      setIsRunning(false);
    }
  }

  async function openBrowserPath(path: string) {
    setError(null);
    try {
      await openPath(path);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  }

  const disabled = !client || !threadId || isRunning;
  return (
    <section className="browser-panel" aria-label="Browser">
      <div className="browser-address-row">
        <input
          aria-label="Browser URL"
          autoCapitalize="none"
          autoCorrect="off"
          placeholder="https://"
          spellCheck={false}
          value={url}
          onChange={(event) => setUrl(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") void run("navigate");
          }}
        />
        <button
          aria-label="Navigate"
          className="icon-button"
          disabled={disabled || !url.trim()}
          onClick={() => void run("navigate")}
          title="Navigate"
          type="button"
        >
          {isRunning ? (
            <Loader2 className="spin" size={16} />
          ) : (
            <RefreshCw size={16} />
          )}
        </button>
      </div>

      <div className="browser-toolbar">
        <button
          aria-label="Page snapshot"
          className="icon-button small"
          disabled={disabled}
          onClick={() => void run("snapshot")}
          title="Page snapshot"
          type="button"
        >
          <RefreshCw size={14} />
        </button>
        <button
          aria-label="Capture screenshot"
          className="icon-button small"
          disabled={disabled}
          onClick={() => void run("screenshot")}
          title="Capture screenshot"
          type="button"
        >
          <Camera size={14} />
        </button>
        <button
          aria-label="Download current URL"
          className="icon-button small"
          disabled={disabled || !url.trim()}
          onClick={() => void run("download")}
          title="Download current URL"
          type="button"
        >
          <Download size={14} />
        </button>
        <button
          aria-label="Close browser session"
          className="icon-button small danger"
          disabled={disabled}
          onClick={() => void run("close")}
          title="Close browser session"
          type="button"
        >
          <Square size={13} fill="currentColor" />
        </button>
      </div>

      <div className="browser-selector-row">
        <input
          aria-label="CSS selector"
          placeholder="CSS selector"
          spellCheck={false}
          value={selector}
          onChange={(event) => setSelector(event.target.value)}
        />
        <button
          aria-label="Click selected element"
          className="icon-button small"
          disabled={disabled || !selector.trim()}
          onClick={() => void run("click")}
          title="Click selected element"
          type="button"
        >
          <MousePointer2 size={14} />
        </button>
      </div>
      <div className="browser-selector-row">
        <input
          aria-label="Text to type"
          placeholder="Text to type"
          value={text}
          onChange={(event) => setText(event.target.value)}
        />
        <button
          aria-label="Type into selected element"
          className="icon-button small"
          disabled={disabled || !selector.trim() || !text.length}
          onClick={() => void run("type")}
          title="Type into selected element"
          type="button"
        >
          <Keyboard size={14} />
        </button>
      </div>

      {error && (
        <p className="browser-error" role="alert">
          {error}
        </p>
      )}
      {pendingBrowserApproval?.type === "approval_requested" && (
        <div className="browser-approval">
          <p>{pendingBrowserApproval.reason}</p>
          <div>
            <button
              className="secondary-button"
              disabled={
                decidingApprovalId === pendingBrowserApproval.approval_id
              }
              onClick={() =>
                onDecideApproval(pendingBrowserApproval.approval_id, false)
              }
              type="button"
            >
              Deny
            </button>
            <button
              className="primary-button"
              disabled={
                decidingApprovalId === pendingBrowserApproval.approval_id
              }
              onClick={() =>
                onDecideApproval(pendingBrowserApproval.approval_id, true)
              }
              type="button"
            >
              Allow Domain
            </button>
          </div>
        </div>
      )}
      {screenshot && (
        <img
          alt="Browser screenshot"
          className="browser-screenshot"
          src={browserImageUrl(screenshot.mime_type, screenshot.bytes)}
        />
      )}
      {snapshotText && <pre className="browser-snapshot">{snapshotText}</pre>}
      {downloads.length > 0 && (
        <div className="browser-downloads">
          {downloads.map((download) =>
            download.type === "file" ? (
              <button
                aria-label={`Open ${download.path}`}
                className="browser-download-path"
                key={download.path}
                onClick={() => void openBrowserPath(download.path)}
                title={`Open ${download.path}`}
                type="button"
              >
                <FolderOpen size={13} />
                <code>{download.path}</code>
              </button>
            ) : null,
          )}
        </div>
      )}
    </section>
  );
}

function browserImageUrl(mimeType: string, bytes: number[]): string {
  let binary = "";
  const chunkSize = 0x8000;
  for (let index = 0; index < bytes.length; index += chunkSize) {
    binary += String.fromCharCode(...bytes.slice(index, index + chunkSize));
  }
  return `data:${mimeType};base64,${btoa(binary)}`;
}

function isBrowserToolResult(result: ToolResult): boolean {
  return asRecord(result.metadata)?.toolName === "browser";
}

function browserOutputFromToolResult(result: ToolResult): BrowserOutput {
  const metadata = asRecord(result.metadata);
  const parts: ModelContentPart[] = result.content?.length
    ? result.content
    : result.output
      ? [{ type: "text", text: result.output }]
      : [];
  const textTruncated = parts.some(
    (part) =>
      part.type === "json" && asRecord(part.value)?.textTruncated === true,
  );

  return {
    url: typeof metadata?.url === "string" ? metadata.url : null,
    contents: parts.map((part) =>
      browserContentFromModelPart(part, textTruncated),
    ),
    metadata: metadata?.browser ?? result.metadata,
  };
}

function browserContentFromModelPart(
  part: ModelContentPart,
  textTruncated: boolean,
): BrowserContent {
  switch (part.type) {
    case "text":
      return { type: "text", text: part.text, truncated: textTruncated };
    case "json":
      return { type: "json", value: part.value };
    case "image":
      return {
        type: "image",
        mime_type: part.content_type,
        bytes: part.data,
      };
    case "resource":
      return {
        type: "file",
        path: browserResourcePath(part.uri),
        mime_type: part.content_type,
        bytes: 0,
      };
  }
}

function browserToolError(result: ToolResult): string | null {
  const metadata = asRecord(result.metadata);
  if (metadata?.success !== false && metadata?.isError !== true) return null;
  return typeof metadata.error === "string" ? metadata.error : result.output;
}

function browserResourcePath(uri: string): string {
  if (!uri.toLocaleLowerCase().startsWith("file:")) return uri;
  try {
    const url = new URL(uri);
    const decodedPath = decodeURIComponent(url.pathname);
    const withoutWindowsPrefix = /^\/[a-z]:/i.test(decodedPath)
      ? decodedPath.slice(1)
      : decodedPath;
    return url.host
      ? `//${url.host}${withoutWindowsPrefix}`
      : withoutWindowsPrefix;
  } catch {
    return uri;
  }
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}
