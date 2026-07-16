import { useCallback, useEffect, useRef, useState } from "react";
import {
  ArrowLeft,
  ArrowRight,
  ExternalLink,
  Loader2,
  RefreshCw,
} from "lucide-react";
import type { ApiClient } from "../api/client";
import { openExternal } from "../platform";
import type { AgentEvent, WebPreviewState } from "../types";
import { BrowserPanel } from "./BrowserPanel";

export function WebPreviewSurface({
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
  const nativeApi = window.opentopia?.browserHost;
  if (!nativeApi) {
    return (
      <BrowserPanel
        client={client}
        threadId={threadId}
        events={events}
        pendingApprovalIds={pendingApprovalIds}
        decidingApprovalId={decidingApprovalId}
        onDecideApproval={onDecideApproval}
      />
    );
  }

  const pendingBrowserApproval = [...events]
    .reverse()
    .find(
      (event) =>
        event.payload.type === "approval_requested" &&
        event.payload.action.startsWith("browser:domain:") &&
        pendingApprovalIds.includes(event.payload.approval_id),
    );
  const approval =
    pendingBrowserApproval?.payload.type === "approval_requested"
      ? pendingBrowserApproval.payload
      : null;

  return (
    <NativeWebPreview
      threadId={threadId}
      approval={approval}
      decidingApprovalId={decidingApprovalId}
      onDecideApproval={onDecideApproval}
    />
  );
}

function NativeWebPreview({
  threadId,
  approval,
  decidingApprovalId,
  onDecideApproval,
}: {
  threadId: string | null;
  approval: Extract<
    AgentEvent["payload"],
    { type: "approval_requested" }
  > | null;
  decidingApprovalId: string | null;
  onDecideApproval(approvalId: string, approved: boolean): void;
}) {
  const api = window.opentopia!.browserHost!;
  const containerRef = useRef<HTMLDivElement>(null);
  const sessionId = threadId ?? "";
  const [address, setAddress] = useState("");
  const [state, setState] = useState<WebPreviewState>({
    sessionId,
    url: "",
    loading: false,
    canGoBack: false,
    canGoForward: false,
  });
  const [error, setError] = useState<string | null>(null);
  const visibleRef = useRef(true);

  const reportBounds = useCallback(() => {
    const element = containerRef.current;
    if (!element) return;
    const rect = element.getBoundingClientRect();
    const visible =
      visibleRef.current &&
      !approval &&
      document.visibilityState === "visible" &&
      rect.width > 0 &&
      rect.height > 0;
    if (!sessionId) return;
    const bounds = {
      x: Math.round(rect.x),
      y: Math.round(rect.y),
      width: Math.max(0, Math.round(rect.width)),
      height: Math.max(0, Math.round(rect.height)),
    };
    void Promise.all([
      api.setBounds(sessionId, bounds),
      api.setVisibility(sessionId, visible),
    ]).catch((cause) => setError(errorMessage(cause)));
  }, [api, approval, sessionId]);

  useEffect(() => {
    if (!threadId) {
      setError("Browser preview requires an active task.");
      return;
    }
    let disposed = false;
    setError(null);
    setState({
      sessionId,
      url: "",
      loading: false,
      canGoBack: false,
      canGoForward: false,
    });
    void api
      .createSession({ sessionId, visible: false })
      .then((next) => {
        if (disposed) return;
        setState(next);
        setAddress(next.url);
        window.requestAnimationFrame(reportBounds);
      })
      .catch((cause) => {
        if (!disposed) setError(errorMessage(cause));
      });
    const unsubscribe = api.onStateChanged((next) => {
      if (next.sessionId !== sessionId || disposed) return;
      setState(next);
      setAddress(next.url);
      setError(next.error ?? null);
    });
    return () => {
      disposed = true;
      unsubscribe?.();
      void api.hide(sessionId).catch(() => {});
    };
  }, [api, reportBounds, sessionId, threadId]);

  useEffect(() => {
    const element = containerRef.current;
    if (!element) return;
    const resizeObserver = new ResizeObserver(reportBounds);
    const intersectionObserver = new IntersectionObserver((entries) => {
      visibleRef.current = entries[0]?.isIntersecting ?? false;
      reportBounds();
    });
    const handleVisibility = () => reportBounds();
    const handleWindowChange = () => reportBounds();
    resizeObserver.observe(element);
    intersectionObserver.observe(element);
    document.addEventListener("visibilitychange", handleVisibility);
    window.addEventListener("resize", handleWindowChange);
    window.addEventListener("scroll", handleWindowChange, true);
    reportBounds();
    return () => {
      visibleRef.current = false;
      if (sessionId) void api.hide(sessionId).catch(() => {});
      resizeObserver.disconnect();
      intersectionObserver.disconnect();
      document.removeEventListener("visibilitychange", handleVisibility);
      window.removeEventListener("resize", handleWindowChange);
      window.removeEventListener("scroll", handleWindowChange, true);
    };
  }, [api, reportBounds, sessionId]);

  async function navigate() {
    try {
      const url = normalizeWebUrl(address);
      setError(null);
      setAddress(url);
      await api.navigate(sessionId, url);
    } catch (cause) {
      setError(errorMessage(cause));
    }
  }

  async function run(action: "back" | "forward" | "reload") {
    setError(null);
    try {
      if (action === "back") await api.back(sessionId);
      else if (action === "forward") await api.forward(sessionId);
      else await api.reload(sessionId);
    } catch (cause) {
      setError(errorMessage(cause));
    }
  }

  return (
    <section className="web-preview" aria-label="Web preview">
      <div className="web-preview-toolbar">
        <button
          className="icon-button small"
          type="button"
          title="Back"
          aria-label="Go back"
          disabled={!state.canGoBack}
          onClick={() => void run("back")}
        >
          <ArrowLeft size={14} />
        </button>
        <button
          className="icon-button small"
          type="button"
          title="Forward"
          aria-label="Go forward"
          disabled={!state.canGoForward}
          onClick={() => void run("forward")}
        >
          <ArrowRight size={14} />
        </button>
        <button
          className="icon-button small"
          type="button"
          title="Reload"
          aria-label="Reload page"
          onClick={() => void run("reload")}
        >
          {state.loading ? (
            <Loader2 className="spin" size={14} />
          ) : (
            <RefreshCw size={14} />
          )}
        </button>
        <input
          aria-label="Web address"
          autoCapitalize="none"
          autoCorrect="off"
          spellCheck={false}
          value={address}
          placeholder="https://"
          onChange={(event) => setAddress(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") void navigate();
          }}
        />
        <button
          className="icon-button small"
          type="button"
          title="Open in default browser"
          aria-label="Open in default browser"
          disabled={!state.url}
          onClick={() => state.url && void openExternal(state.url)}
        >
          <ExternalLink size={14} />
        </button>
      </div>
      {error && (
        <div className="web-preview-error" role="alert">
          {error}
        </div>
      )}
      {approval && (
        <div className="web-preview-approval" role="alert">
          <div>
            <strong>Allow this domain?</strong>
            <span>{approval.reason}</span>
          </div>
          <button
            className="secondary-button compact"
            disabled={decidingApprovalId === approval.approval_id}
            onClick={() => onDecideApproval(approval.approval_id, false)}
            type="button"
          >
            Deny
          </button>
          <button
            className="primary-button compact"
            disabled={decidingApprovalId === approval.approval_id}
            onClick={() => onDecideApproval(approval.approval_id, true)}
            type="button"
          >
            Allow
          </button>
        </div>
      )}
      <div className="web-preview-native-surface" ref={containerRef} />
    </section>
  );
}

function normalizeWebUrl(value: string): string {
  const candidate = /^https?:\/\//i.test(value.trim())
    ? value.trim()
    : `https://${value.trim()}`;
  const parsed = new URL(candidate);
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
    throw new Error("Only HTTP and HTTPS URLs can be previewed.");
  }
  return parsed.toString();
}

function errorMessage(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}
