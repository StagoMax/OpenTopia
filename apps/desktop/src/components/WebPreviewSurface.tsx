import { useCallback, useEffect, useRef, useState } from "react";
import {
  ArrowLeft,
  ArrowRight,
  Cable,
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
  BrowserRuntimeRoute,
  ChromeBridgeState,
  WebPreviewState,
} from "../types";
import { activeBrowserHandoff, type BrowserHandoff } from "../browserHandoff";
import { resolveAddressBarInput } from "../browserNavigation";
import { BrowserPanel } from "./BrowserPanel";
import { SegmentedControl } from "./ui/SegmentedControl";

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
      key={threadId ?? "no-active-thread"}
      client={client}
      threadId={threadId}
      handoff={handoff}
      navigationRequest={navigationRequest}
    />
  );
}

function NativeWebPreview({
  client,
  threadId,
  handoff,
  navigationRequest,
}: {
  client: ApiClient | null;
  threadId: string | null;
  handoff: BrowserHandoff | null;
  navigationRequest: BrowserNavigationRequest | null;
}) {
  const api = window.opentopia!.browserHost!;
  const chromeApi = window.opentopia?.chromeBridge;
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
  const [runtimeRoute, setRuntimeRoute] =
    useState<BrowserRuntimeRoute>("managed");
  const [runtimeBusy, setRuntimeBusy] = useState(false);
  const [chromeAvailable, setChromeAvailable] = useState(false);
  const [chromeState, setChromeState] = useState<ChromeBridgeState | null>(
    null,
  );
  const [error, setError] = useState<string | null>(null);
  const runtimeRouteRef = useRef<BrowserRuntimeRoute>("managed");
  const visibleRef = useRef(true);
  const hasUrlRef = useRef(false);
  const handledNavigationIdRef = useRef<string | null>(null);

  useEffect(() => {
    if (!sessionId) return;
    let disposed = false;
    const applyChromeState = (next: ChromeBridgeState) => {
      if (disposed || next.sessionId !== sessionId) return;
      setChromeState(next);
      if (runtimeRouteRef.current === "chrome") {
        setAddress(next.url);
        setError(next.error ?? null);
      }
    };
    const unsubscribe = chromeApi?.onStateChanged(applyChromeState);
    void (async () => {
      if (client && threadId) {
        const runtime = await client.getBrowserRuntime(threadId);
        if (disposed) return;
        runtimeRouteRef.current = runtime.route;
        setRuntimeRoute(runtime.route);
        setChromeAvailable(runtime.chromeAvailable && Boolean(chromeApi));
      }
      if (chromeApi) applyChromeState(await chromeApi.getStatus(sessionId));
    })().catch((cause) => {
      if (!disposed) setError(errorMessage(cause));
    });
    return () => {
      disposed = true;
      unsubscribe?.();
    };
  }, [chromeApi, client, sessionId, threadId]);

  useEffect(() => {
    if (
      runtimeRoute !== "chrome" ||
      chromeState?.status !== "attached" ||
      !client ||
      !threadId
    ) {
      return;
    }
    let disposed = false;
    setRuntimeBusy(true);
    void client
      .bindBrowserRuntime(threadId, "chrome")
      .then((runtime) => {
        if (!disposed) setChromeAvailable(runtime.chromeAvailable);
      })
      .catch((cause) => {
        if (!disposed) setError(errorMessage(cause));
      })
      .finally(() => {
        if (!disposed) setRuntimeBusy(false);
      });
    return () => {
      disposed = true;
    };
  }, [chromeState?.status, chromeState?.tabId, client, runtimeRoute, threadId]);

  useEffect(() => {
    if (!handoff || !sessionId || runtimeRoute !== "managed") return;
    void api
      .createSession({ sessionId })
      .then(() => api.beginUserControl(sessionId))
      .catch((cause) => setError(errorMessage(cause)));
  }, [api, handoff, runtimeRoute, sessionId]);

  const reportBounds = useCallback(() => {
    const element = containerRef.current;
    if (!element) return;
    const rect = element.getBoundingClientRect();
    const visible =
      runtimeRouteRef.current === "managed" &&
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
    if (runtimeRoute !== "managed") {
      hasUrlRef.current = false;
      void api.hide(sessionId).catch(() => {});
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
  }, [api, reportBounds, runtimeRoute, sessionId, threadId]);

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
    const navigation =
      runtimeRoute === "chrome"
        ? chromeApi?.runAction(sessionId, "navigate", navigationRequest.url)
        : api
            .createSession({ sessionId, visible: false })
            .then(() => api.navigate(sessionId, navigationRequest.url));
    void Promise.resolve(navigation)
      .then(() => {
        if (
          !disposed &&
          runtimeRoute === "chrome" &&
          chromeState?.status !== "attached"
        ) {
          throw new Error("请先在 Chrome 扩展中连接当前标签页。");
        }
      })
      .catch((cause) => {
        if (!disposed) setError(errorMessage(cause));
      });
    return () => {
      disposed = true;
    };
  }, [
    api,
    chromeApi,
    chromeState?.status,
    navigationRequest,
    runtimeRoute,
    sessionId,
  ]);

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
      const url = resolveAddressBarInput(address);
      setError(null);
      setAddress(url);
      if (runtimeRoute === "chrome") {
        if (!chromeApi || chromeState?.status !== "attached") {
          throw new Error("请先在 Chrome 扩展中连接当前标签页。");
        }
        await chromeApi.runAction(sessionId, "navigate", url);
      } else {
        await api.navigateFromAddressBar(sessionId, url);
      }
    } catch (cause) {
      setError(errorMessage(cause));
    }
  }

  async function run(action: "back" | "forward" | "reload") {
    setError(null);
    try {
      if (runtimeRoute === "chrome") {
        if (!chromeApi || chromeState?.status !== "attached") {
          throw new Error("请先在 Chrome 扩展中连接当前标签页。");
        }
        await chromeApi.runAction(sessionId, action);
      } else if (action === "back") await api.back(sessionId);
      else if (action === "forward") await api.forward(sessionId);
      else await api.reload(sessionId);
    } catch (cause) {
      setError(errorMessage(cause));
    }
  }

  async function changeRuntime(next: BrowserRuntimeRoute) {
    if (!sessionId || runtimeBusy) return;
    setRuntimeBusy(true);
    setError(null);
    try {
      if (next === "chrome") {
        if (!chromeApi || !chromeAvailable) {
          throw new Error("Chrome 连接服务当前不可用。");
        }
        let nextChromeState = await chromeApi.getStatus(sessionId);
        if (nextChromeState.status === "idle") {
          nextChromeState = await chromeApi.startPairing(sessionId);
        }
        await api.hide(sessionId);
        runtimeRouteRef.current = "chrome";
        setRuntimeRoute("chrome");
        setChromeState(nextChromeState);
        setAddress(nextChromeState.url);
      } else {
        if (client && threadId) {
          const runtime = await client.bindBrowserRuntime(threadId, "managed");
          setChromeAvailable(runtime.chromeAvailable && Boolean(chromeApi));
        }
        if (chromeApi) await chromeApi.disconnect(sessionId);
        runtimeRouteRef.current = "managed";
        setRuntimeRoute("managed");
        setAddress(state.url);
        window.requestAnimationFrame(reportBounds);
      }
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setRuntimeBusy(false);
    }
  }

  const chromeReady =
    runtimeRoute === "managed" || chromeState?.status === "attached";
  const currentUrl =
    runtimeRoute === "chrome" ? (chromeState?.url ?? "") : state.url;
  const currentLoading = runtimeRoute === "managed" && state.loading;

  return (
    <section className="web-preview" aria-label="Web preview">
      <div className="web-preview-toolbar">
        <SegmentedControl
          className="web-preview-runtime-selector"
          label="浏览器运行方式"
          value={runtimeRoute}
          disabled={runtimeBusy}
          options={[
            { value: "managed", label: "内置" },
            {
              value: "chrome",
              label: "Chrome",
              disabled: !chromeAvailable,
            },
          ]}
          onChange={(next) => void changeRuntime(next)}
        />
        <button
          className="icon-button small"
          type="button"
          title="Back"
          aria-label="Go back"
          disabled={
            !chromeReady || (runtimeRoute === "managed" && !state.canGoBack)
          }
          onClick={() => void run("back")}
        >
          <ArrowLeft size={14} />
        </button>
        <button
          className="icon-button small"
          type="button"
          title="Forward"
          aria-label="Go forward"
          disabled={
            !chromeReady || (runtimeRoute === "managed" && !state.canGoForward)
          }
          onClick={() => void run("forward")}
        >
          <ArrowRight size={14} />
        </button>
        <button
          className="icon-button small"
          type="button"
          title="Reload"
          aria-label="Reload page"
          disabled={!chromeReady || !currentUrl}
          onClick={() => void run("reload")}
        >
          {currentLoading ? (
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
          placeholder="输入 URL 或搜索内容"
          disabled={!chromeReady}
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
          disabled={!currentUrl}
          onClick={() => currentUrl && void openExternal(currentUrl)}
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
      <div
        className="web-preview-native-surface"
        ref={containerRef}
        hidden={runtimeRoute !== "managed"}
      />
      {runtimeRoute === "chrome" ? (
        <div
          className="web-preview-chrome-surface"
          role="status"
          aria-live="polite"
        >
          <Cable size={32} aria-hidden="true" />
          {chromeState?.status === "attached" ? (
            <>
              <strong>{chromeState.title || "已连接 Chrome 标签页"}</strong>
              <span>{chromeState.url || "OpenTopia 可以使用此标签页。"}</span>
            </>
          ) : chromeState?.status === "waiting_for_tab" ? (
            <>
              <strong>选择要连接的 Chrome 标签页</strong>
              <span>打开 OpenTopia 扩展，然后点击“连接当前标签页”。</span>
            </>
          ) : chromeState?.status === "waiting_for_extension" ? (
            <>
              <strong>在 Chrome 扩展中输入配对码</strong>
              {chromeState.pairingCode ? (
                <code aria-label={`配对码 ${chromeState.pairingCode}`}>
                  {chromeState.pairingCode}
                </code>
              ) : null}
              <span>配对后，由你明确选择允许 OpenTopia 使用的标签页。</span>
            </>
          ) : (
            <>
              <strong>Chrome 尚未连接</strong>
              <span>重新生成一次性配对码后，在 OpenTopia 扩展中完成连接。</span>
              <button
                className="secondary-button compact"
                type="button"
                disabled={runtimeBusy}
                onClick={() => void changeRuntime("chrome")}
              >
                重新连接
              </button>
            </>
          )}
        </div>
      ) : !state.url ? (
        <div className="web-preview-empty">
          <Globe2 size={32} aria-hidden="true" />
          <strong>开始浏览</strong>
          <span>输入完整网址，或直接使用 Google 搜索</span>
        </div>
      ) : null}
    </section>
  );
}

function errorMessage(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}
