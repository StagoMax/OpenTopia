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
import {
  activeBrowserHandoff,
  activeBrowserHandoffTurnId,
  type BrowserHandoff,
} from "../browserHandoff";
import { syncBrowserAddress } from "../browserAddressState";
import {
  browserSessionId,
  navigateBrowserAddress,
  resolveAddressBarInput,
} from "../browserNavigation";
import { openExternal } from "../platform";
import type {
  AgentEvent,
  BrowserNavigationRequest,
  WebPreviewState,
} from "../types";
import { BrowserPanel } from "./BrowserPanel";
import { Button } from "./ui";

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
  const handoffTurnId = activeBrowserHandoffTurnId(events, threadId);
  const sessionId = browserSessionId(threadId);

  return (
    <NativeWebPreview
      key={sessionId}
      sessionId={sessionId}
      client={client}
      threadId={threadId}
      handoff={handoff}
      handoffTurnId={handoffTurnId}
      navigationRequest={navigationRequest}
    />
  );
}

function NativeWebPreview({
  sessionId,
  client,
  threadId,
  handoff,
  handoffTurnId,
  navigationRequest,
}: {
  sessionId: string;
  client: ApiClient | null;
  threadId: string | null;
  handoff: BrowserHandoff | null;
  handoffTurnId: string | null;
  navigationRequest: BrowserNavigationRequest | null;
}) {
  const api = window.opentopia!.browserHost!;
  const containerRef = useRef<HTMLDivElement>(null);
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
  const [isResuming, setIsResuming] = useState(false);
  const [isCancelling, setIsCancelling] = useState(false);
  const visibleRef = useRef(true);
  const hasUrlRef = useRef(false);
  const handledNavigationIdRef = useRef<string | null>(null);
  const addressValueRef = useRef("");
  const addressEditingRef = useRef(false);
  const addressDirtyRef = useRef(false);
  const browserUrlRef = useRef("");
  const pendingNavigationOriginRef = useRef("");
  const pendingAddressRef = useRef<string | null>(null);
  const lastReportedBoundsRef = useRef<string | null>(null);
  const lastReportedVisibilityRef = useRef<boolean | null>(null);

  useEffect(() => {
    if (!handoff) return;
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
    const bounds = {
      x: Math.round(rect.x),
      y: Math.round(rect.y),
      width: Math.max(0, Math.round(rect.width)),
      height: Math.max(0, Math.round(rect.height)),
    };
    const boundsKey = `${bounds.x}:${bounds.y}:${bounds.width}:${bounds.height}`;
    const shouldUpdateBounds = lastReportedBoundsRef.current !== boundsKey;
    const shouldUpdateVisibility =
      lastReportedVisibilityRef.current !== visible;
    if (!shouldUpdateBounds && !shouldUpdateVisibility) return;

    // Mark values before crossing IPC. The resulting state event must not
    // immediately schedule the same update again.
    if (shouldUpdateBounds) lastReportedBoundsRef.current = boundsKey;
    if (shouldUpdateVisibility) lastReportedVisibilityRef.current = visible;

    void Promise.all([
      shouldUpdateBounds ? api.setBounds(sessionId, bounds) : Promise.resolve(),
      shouldUpdateVisibility
        ? api.setVisibility(sessionId, visible)
        : Promise.resolve(),
    ]).catch((cause) => {
      if (shouldUpdateBounds && lastReportedBoundsRef.current === boundsKey) {
        lastReportedBoundsRef.current = null;
      }
      if (
        shouldUpdateVisibility &&
        lastReportedVisibilityRef.current === visible
      ) {
        lastReportedVisibilityRef.current = null;
      }
      setError(errorMessage(cause));
    });
  }, [api, sessionId]);

  useEffect(() => {
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
    addressValueRef.current = "";
    addressEditingRef.current = false;
    addressDirtyRef.current = false;
    browserUrlRef.current = "";
    pendingNavigationOriginRef.current = "";
    pendingAddressRef.current = null;
    lastReportedBoundsRef.current = null;
    lastReportedVisibilityRef.current = null;
    void api
      .createSession({ sessionId, visible: false })
      .then((next) => {
        if (disposed) return;
        hasUrlRef.current = Boolean(next.url);
        setState(next);
        syncAddress(next);
        window.requestAnimationFrame(reportBounds);
      })
      .catch((cause) => {
        if (!disposed) setError(errorMessage(cause));
      });
    const unsubscribe = api.onStateChanged((next) => {
      if (next.sessionId !== sessionId || disposed) return;
      hasUrlRef.current = Boolean(next.url);
      setState(next);
      syncAddress(next);
      setError(next.error ?? null);
      window.requestAnimationFrame(reportBounds);
    });
    return () => {
      disposed = true;
      unsubscribe?.();
      void api.hide(sessionId).catch(() => {});
    };
  }, [api, reportBounds, sessionId]);

  useEffect(() => {
    if (
      !navigationRequest ||
      handledNavigationIdRef.current === navigationRequest.id
    ) {
      return;
    }
    handledNavigationIdRef.current = navigationRequest.id;
    let disposed = false;
    pendingAddressRef.current = navigationRequest.url;
    pendingNavigationOriginRef.current = browserUrlRef.current;
    addressDirtyRef.current = false;
    setAddressValue(navigationRequest.url);
    setError(null);
    void api
      .createSession({ sessionId, visible: false })
      .then(() => api.navigateFromAddressBar(sessionId, navigationRequest.url))
      .catch((cause) => {
        if (!disposed) {
          pendingAddressRef.current = null;
          pendingNavigationOriginRef.current = "";
          setError(errorMessage(cause));
        }
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
      void api.hide(sessionId).catch(() => {});
      resizeObserver.disconnect();
      intersectionObserver.disconnect();
      document.removeEventListener("visibilitychange", handleVisibility);
      window.removeEventListener("resize", handleWindowChange);
      window.removeEventListener("scroll", handleWindowChange, true);
    };
  }, [api, reportBounds, sessionId]);

  async function navigate() {
    setError(null);
    let requestedUrl: string;
    try {
      requestedUrl = resolveAddressBarInput(addressValueRef.current);
    } catch (cause) {
      setError(errorMessage(cause));
      return;
    }
    pendingAddressRef.current = requestedUrl;
    pendingNavigationOriginRef.current = browserUrlRef.current;
    addressDirtyRef.current = false;
    setAddressValue(requestedUrl);
    try {
      const url = await navigateBrowserAddress(api, sessionId, requestedUrl);
      setAddressValue(url);
    } catch (cause) {
      pendingAddressRef.current = null;
      pendingNavigationOriginRef.current = "";
      setError(errorMessage(cause));
    }
  }

  function setAddressValue(value: string) {
    addressValueRef.current = value;
    setAddress(value);
  }

  function syncAddress(next: WebPreviewState) {
    const hadPendingAddress = pendingAddressRef.current !== null;
    const result = syncBrowserAddress({
      currentValue: addressValueRef.current,
      browserUrl: next.url,
      loading: next.loading,
      error: next.error,
      editing: addressEditingRef.current,
      dirty: addressDirtyRef.current,
      previousBrowserUrl:
        pendingNavigationOriginRef.current || browserUrlRef.current,
      pendingUrl: pendingAddressRef.current,
    });
    if (!result.pendingUrl && hadPendingAddress) {
      addressDirtyRef.current = false;
      pendingNavigationOriginRef.current = "";
    }
    pendingAddressRef.current = result.pendingUrl;
    browserUrlRef.current = next.url;
    setAddressValue(result.value);
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

  async function resumeHandoff() {
    if (!client || !threadId || !handoffTurnId || isResuming || isCancelling)
      return;
    setIsResuming(true);
    setError(null);
    try {
      await client.resumeExternalAction(
        threadId,
        handoffTurnId,
        "用户已在嵌入式浏览器中完成所需操作。",
      );
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setIsResuming(false);
    }
  }

  async function cancelHandoff() {
    if (!client || !threadId || !handoffTurnId || isResuming || isCancelling)
      return;
    setIsCancelling(true);
    setError(null);
    try {
      const result = await client.cancelTurn(threadId, handoffTurnId);
      if (!result.cancelled) {
        throw new Error(result.message || "服务端未取消等待中的任务。");
      }
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setIsCancelling(false);
    }
  }

  return (
    <section className="web-preview" aria-label="浏览器">
      <div className="web-preview-toolbar">
        <button
          className="icon-button small"
          type="button"
          title="后退"
          aria-label="后退"
          disabled={!state.canGoBack}
          onClick={() => void run("back")}
        >
          <ArrowLeft size={14} />
        </button>
        <button
          className="icon-button small"
          type="button"
          title="前进"
          aria-label="前进"
          disabled={!state.canGoForward}
          onClick={() => void run("forward")}
        >
          <ArrowRight size={14} />
        </button>
        <button
          className="icon-button small"
          type="button"
          title="重新加载"
          aria-label="重新加载"
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
          aria-label="网址或搜索内容"
          autoCapitalize="none"
          autoCorrect="off"
          spellCheck={false}
          onBlur={() => {
            addressEditingRef.current = false;
            addressDirtyRef.current = false;
          }}
          onFocus={() => {
            addressEditingRef.current = true;
          }}
          value={address}
          placeholder="输入网址或搜索内容"
          onChange={(event) => {
            pendingAddressRef.current = null;
            addressDirtyRef.current = true;
            setAddressValue(event.target.value);
          }}
          onKeyDown={(event) => {
            if (event.key === "Enter") void navigate();
          }}
        />
        <button
          className="icon-button small"
          type="button"
          title="在默认浏览器中打开"
          aria-label="在默认浏览器中打开"
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
          <Button
            size="compact"
            variant="primary"
            disabled={
              !client ||
              !threadId ||
              !handoffTurnId ||
              isResuming ||
              isCancelling
            }
            onClick={() => void resumeHandoff()}
          >
            {isResuming ? "正在继续…" : "已完成，继续执行"}
          </Button>
          <Button
            size="compact"
            variant="quiet"
            disabled={
              !client ||
              !threadId ||
              !handoffTurnId ||
              isResuming ||
              isCancelling
            }
            onClick={() => void cancelHandoff()}
          >
            {isCancelling ? "正在结束…" : "结束本轮"}
          </Button>
        </div>
      )}
      <div className="web-preview-native-surface" ref={containerRef} />
      {!state.url ? (
        <div className="web-preview-empty">
          <Globe2 size={32} aria-hidden="true" />
          <strong>开始浏览</strong>
          <span>输入网址，或直接使用 Google 搜索</span>
        </div>
      ) : null}
    </section>
  );
}

function errorMessage(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}
