import { useCallback, useEffect, useRef, useState } from "react";
import {
  ArrowLeft,
  ArrowRight,
  ExternalLink,
  Globe2,
  Loader2,
  RefreshCw,
} from "lucide-react";
import type { ApiClient } from "../api/client";
import { openExternal } from "../platform";
import type {
  AgentEvent,
  BrowserNavigationRequest,
  WebPreviewState,
} from "../types";
import { activeBrowserHandoff, type BrowserHandoff } from "../browserHandoff";
import { BrowserPanel } from "./BrowserPanel";

export function WebPreviewSurface({
  client,
  threadId,
  events,
  navigationRequest,
}: {
  client: ApiClient | null;
  threadId: string | null;
  events: AgentEvent[];
  navigationRequest: BrowserNavigationRequest | null;
}) {
  const nativeApi = window.opentopia?.browserHost;
  if (!nativeApi) {
    return (
      <BrowserPanel
        client={client}
        threadId={threadId}
        events={events}
        navigationRequest={navigationRequest}
      />
    );
  }

  const handoff = activeBrowserHandoff(events, threadId);

  return (
    <NativeWebPreview
      threadId={threadId}
      handoff={handoff}
      navigationRequest={navigationRequest}
    />
  );
}

function NativeWebPreview({
  threadId,
  handoff,
  navigationRequest,
}: {
  threadId: string | null;
  handoff: BrowserHandoff | null;
  navigationRequest: BrowserNavigationRequest | null;
}) {
  const api = window.opentopia!.browserHost!;
  const containerRef = useRef<HTMLDivElement>(null);
  const sessionId = threadId ?? "";
  const [address, setAddress] = useState("");
  const [state, setState] = useState<WebPreviewState>({
    sessionId,
    profileId: "default",
    profilePersistence: "persistent",
    url: "",
    loading: false,
    canGoBack: false,
    canGoForward: false,
  });
  const [error, setError] = useState<string | null>(null);
  const visibleRef = useRef(true);
  const hasUrlRef = useRef(false);
  const handledNavigationIdRef = useRef<string | null>(null);

  useEffect(() => {
    if (!handoff || !sessionId) return;
    void api
      .createSession({ sessionId })
      .then(() => api.beginUserControl(sessionId))
      .catch((cause) => setError(errorMessage(cause)));
  }, [api, handoff, sessionId]);

  const reportBounds = useCallback(() => {
    const element = containerRef.current;
    if (!element) return;
    const rect = element.getBoundingClientRect();
    const visible =
      visibleRef.current &&
      hasUrlRef.current &&
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
  }, [api, sessionId]);

  useEffect(() => {
    if (!threadId) {
      setError("Browser preview requires an active task.");
      return;
    }
    let disposed = false;
    setError(null);
    setState({
      sessionId,
      profileId: "default",
      profilePersistence: "persistent",
      url: "",
      loading: false,
      canGoBack: false,
      canGoForward: false,
    });
    hasUrlRef.current = false;
    void api
      .createSession({ sessionId, visible: false })
      .then((next) => {
        if (disposed) return;
        hasUrlRef.current = Boolean(next.url);
        setState(next);
        setAddress(next.url);
        window.requestAnimationFrame(reportBounds);
      })
      .catch((cause) => {
        if (!disposed) setError(errorMessage(cause));
      });
    const unsubscribe = api.onStateChanged((next) => {
      if (next.sessionId !== sessionId || disposed) return;
      hasUrlRef.current = Boolean(next.url);
      setState(next);
      setAddress(next.url);
      setError(next.error ?? null);
      window.requestAnimationFrame(reportBounds);
    });
    return () => {
      disposed = true;
      unsubscribe?.();
      void api.hide(sessionId).catch(() => {});
    };
  }, [api, reportBounds, sessionId, threadId]);

  useEffect(() => {
    if (
      !navigationRequest ||
      !sessionId ||
      handledNavigationIdRef.current === navigationRequest.id
    ) {
      return;
    }
    handledNavigationIdRef.current = navigationRequest.id;
    let disposed = false;
    setAddress(navigationRequest.url);
    setError(null);
    void api
      .createSession({ sessionId, visible: false })
      .then(() => {
        if (disposed) return undefined;
        return api.navigate(sessionId, navigationRequest.url);
      })
      .catch((cause) => {
        if (!disposed) setError(errorMessage(cause));
      });
    return () => {
      disposed = true;
    };
  }, [api, navigationRequest, sessionId]);

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
      await api.navigateFromAddressBar(sessionId, url);
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
          disabled={!state.url}
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
          placeholder="输入 URL"
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
      {handoff && (
        <div className="web-preview-handoff" role="status">
          <strong>需要你在当前页面完成操作</strong>
          <span>{handoff.reason}</span>
          <span>完成后在对话中告诉我继续。</span>
        </div>
      )}
      <div className="web-preview-native-surface" ref={containerRef} />
      {!state.url ? (
        <div className="web-preview-empty">
          <Globe2 size={32} aria-hidden="true" />
          <strong>开始浏览</strong>
          <span>输入 URL 以打开页面</span>
        </div>
      ) : null}
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
