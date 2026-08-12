import { Check, Copy } from "lucide-react";
import { useEffect, useId, useRef, useState } from "react";
import { Button } from "./ui/Button";

type RenderState =
  | { source: string; status: "loading"; svg: null }
  | { source: string; status: "ready"; svg: string }
  | { source: string; status: "error"; svg: null };

type CopyStatus = "idle" | "copied" | "error";

let mermaidModulePromise: Promise<typeof import("mermaid").default> | null =
  null;
let mermaidRenderSequence = 0;

function loadMermaid() {
  mermaidModulePromise ??= import("mermaid").then((module) => module.default);
  return mermaidModulePromise;
}

export function MermaidDiagram({ source }: { source: string }) {
  const diagramId = `opentopia-mermaid-${useId().replaceAll(":", "")}`;
  const themeRevision = useThemeRevision();
  const [renderState, setRenderState] = useState<RenderState>({
    source,
    status: "loading",
    svg: null,
  });
  const [copyStatus, setCopyStatus] = useState<CopyStatus>("idle");
  const copyResetTimerRef = useRef<number | null>(null);

  useEffect(() => {
    let disposed = false;
    setRenderState({ source, status: "loading", svg: null });

    void loadMermaid()
      .then(async (mermaid) => {
        const rootStyles = window.getComputedStyle(document.documentElement);
        const token = (name: string) =>
          rootStyles.getPropertyValue(name).trim();

        mermaid.initialize({
          startOnLoad: false,
          securityLevel: "strict",
          suppressErrorRendering: true,
          theme: "base",
          fontFamily: token("--font-sans"),
          htmlLabels: false,
          themeVariables: {
            background: token("--surface"),
            primaryColor: token("--surface-subtle"),
            primaryTextColor: token("--text"),
            primaryBorderColor: token("--border-strong"),
            lineColor: token("--text-secondary"),
            secondaryColor: token("--accent-subtle"),
            secondaryTextColor: token("--text"),
            secondaryBorderColor: token("--accent"),
            tertiaryColor: token("--surface"),
            tertiaryTextColor: token("--text"),
            tertiaryBorderColor: token("--border"),
            noteBkgColor: token("--warning-subtle"),
            noteTextColor: token("--text"),
            noteBorderColor: token("--warning"),
          },
        });

        const renderId = `${diagramId}-${++mermaidRenderSequence}`;
        const { svg } = await mermaid.render(renderId, source);
        if (!disposed) {
          setRenderState({ source, status: "ready", svg });
        }
      })
      .catch(() => {
        if (!disposed) {
          setRenderState({ source, status: "error", svg: null });
        }
      });

    return () => {
      disposed = true;
    };
  }, [diagramId, source, themeRevision]);

  useEffect(
    () => () => {
      if (copyResetTimerRef.current !== null) {
        window.clearTimeout(copyResetTimerRef.current);
      }
    },
    [],
  );

  const activeState: RenderState =
    renderState.source === source
      ? renderState
      : { source, status: "loading", svg: null };
  const copyLabel =
    copyStatus === "copied"
      ? "已复制"
      : copyStatus === "error"
        ? "复制失败"
        : "复制";

  return (
    <figure className="mermaid-diagram">
      <div className="mermaid-diagram__toolbar">
        <Button
          className="mermaid-diagram__copy"
          data-state={copyStatus}
          size="compact"
          variant="quiet"
          onClick={() => {
            void copyMermaidSource(source)
              .then(() => setCopyStatus("copied"))
              .catch(() => setCopyStatus("error"))
              .finally(() => {
                if (copyResetTimerRef.current !== null) {
                  window.clearTimeout(copyResetTimerRef.current);
                }
                copyResetTimerRef.current = window.setTimeout(() => {
                  setCopyStatus("idle");
                  copyResetTimerRef.current = null;
                }, 1600);
              });
          }}
        >
          {copyStatus === "copied" ? (
            <Check size={14} aria-hidden="true" />
          ) : (
            <Copy size={14} aria-hidden="true" />
          )}
          {copyLabel}
        </Button>
      </div>

      {activeState.status === "ready" ? (
        <div
          aria-label="Mermaid 图表"
          className="mermaid-diagram__canvas"
          role="img"
          dangerouslySetInnerHTML={{ __html: activeState.svg }}
        />
      ) : activeState.status === "error" ? (
        <div className="mermaid-diagram__fallback" role="alert">
          <p>图表无法显示，以下是 Mermaid 源码。</p>
          <pre>
            <code>{source}</code>
          </pre>
        </div>
      ) : (
        <div aria-busy="true" className="mermaid-diagram__status" role="status">
          正在绘制图表…
        </div>
      )}

      <span aria-live="polite" className="sr-only">
        {copyStatus === "copied"
          ? "Mermaid 源码已复制到剪贴板"
          : copyStatus === "error"
            ? "Mermaid 源码复制失败"
            : ""}
      </span>
    </figure>
  );
}

async function copyMermaidSource(source: string): Promise<void> {
  if (!navigator.clipboard?.writeText) {
    throw new Error("Clipboard API unavailable");
  }
  await navigator.clipboard.writeText(source);
}

function useThemeRevision(): number {
  const [revision, setRevision] = useState(0);

  useEffect(() => {
    const observer = new MutationObserver(() => {
      setRevision((value) => value + 1);
    });
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["data-theme"],
    });
    return () => observer.disconnect();
  }, []);

  return revision;
}
