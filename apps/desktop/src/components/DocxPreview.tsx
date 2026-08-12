import { useEffect, useRef, useState } from "react";
import { AlertCircle, FileQuestion, Loader2 } from "lucide-react";
import type { ApiClient } from "../api/client";
import type { PreviewDescriptor } from "../types";
import shadowStyles from "./DocxPreview.shadow.css?inline";

type LoadState =
  | { status: "loading" }
  | { status: "ready" }
  | { status: "error"; message: string };

export function DocxPreview({
  client,
  descriptor,
}: {
  client: ApiClient;
  descriptor: PreviewDescriptor;
}) {
  const hostRef = useRef<HTMLDivElement>(null);
  const [state, setState] = useState<LoadState>({ status: "loading" });

  useEffect(() => {
    const controller = new AbortController();
    let disposed = false;
    const host = hostRef.current;
    if (!host) return;
    const shadow = host.shadowRoot ?? host.attachShadow({ mode: "open" });
    const body = document.createElement("div");
    const styles = document.createElement("div");
    body.setAttribute("role", "document");
    body.setAttribute("aria-label", `${descriptor.title} document preview`);
    shadow.replaceChildren(styles, body);
    setState({ status: "loading" });

    void Promise.all([
      client.getPreviewContent(descriptor.threadId, descriptor.id),
      import("docx-preview"),
    ])
      .then(async ([blob, docx]) => {
        if (disposed) return;
        await docx.renderAsync(blob, body, styles, {
          className: "opentopia-docx",
          inWrapper: true,
          breakPages: true,
          renderHeaders: true,
          renderFooters: true,
          renderFootnotes: true,
          renderEndnotes: true,
          renderComments: false,
          renderChanges: false,
          renderAltChunks: false,
          useBase64URL: true,
          ignoreLastRenderedPageBreak: true,
        });
        if (disposed) return;
        hardenRenderedDocument(body);
        const overrides = document.createElement("style");
        overrides.textContent = shadowStyles;
        shadow.appendChild(overrides);
        setState({ status: "ready" });
      })
      .catch((cause) => {
        if (!disposed && !controller.signal.aborted) {
          setState({ status: "error", message: errorMessage(cause) });
        }
      });

    return () => {
      disposed = true;
      controller.abort();
      shadow.replaceChildren();
    };
  }, [client, descriptor.id, descriptor.revision, descriptor.threadId]);

  return (
    <div className="docx-preview" aria-busy={state.status === "loading"}>
      {state.status !== "ready" && <DocumentPreviewStatus state={state} />}
      <div
        ref={hostRef}
        className="docx-preview-content"
        hidden={state.status !== "ready"}
      />
    </div>
  );
}

function hardenRenderedDocument(root: HTMLElement): void {
  for (const link of root.querySelectorAll("a")) {
    link.removeAttribute("href");
    link.removeAttribute("target");
    link.removeAttribute("rel");
  }
  for (const element of root.querySelectorAll(
    "iframe, object, embed, script",
  )) {
    element.remove();
  }
}

function DocumentPreviewStatus({ state }: { state: LoadState }) {
  const loading = state.status === "loading";
  return (
    <div className="preview-status" role={loading ? "status" : "alert"}>
      {loading ? (
        <Loader2 className="spin" size={22} />
      ) : state.status === "error" ? (
        <AlertCircle size={22} />
      ) : (
        <FileQuestion size={22} />
      )}
      <strong>
        {loading ? "Loading document" : "Could not render document"}
      </strong>
      {state.status === "error" && <p>{state.message}</p>}
    </div>
  );
}

function errorMessage(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}
