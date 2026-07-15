import { useMemo, useState } from "react";
import {
  Camera,
  Download,
  Keyboard,
  Loader2,
  MousePointer2,
  RefreshCw,
  Square,
} from "lucide-react";
import { ApiClient } from "../api/client";
import type { AgentEvent, BrowserContent, BrowserOutput } from "../types";

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
      setOutput(next);
      if (next.url) setUrl(next.url);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setIsRunning(false);
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
              disabled={decidingApprovalId === pendingBrowserApproval.approval_id}
              onClick={() =>
                onDecideApproval(pendingBrowserApproval.approval_id, false)
              }
              type="button"
            >
              Deny
            </button>
            <button
              className="primary-button"
              disabled={decidingApprovalId === pendingBrowserApproval.approval_id}
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
              <span key={download.path} title={download.path}>
                <Download size={13} />
                {download.path.split(/[\\/]/).at(-1)}
              </span>
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
