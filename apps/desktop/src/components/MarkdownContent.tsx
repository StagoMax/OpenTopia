import { FileText } from "lucide-react";
import {
  createContext,
  memo,
  useContext,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type AnchorHTMLAttributes,
  type ImgHTMLAttributes,
} from "react";
import ReactMarkdown, {
  defaultUrlTransform,
  type Components,
  type Options,
} from "react-markdown";
import remarkGfm from "remark-gfm";
import {
  renderedTextChange,
  rendererTraceTime,
  type ConversationMarkdownTraceContext,
  type ConversationRenderTrace,
} from "../conversationRenderTrace";
import {
  decodeFilePathHref,
  FILE_PATH_LINK_SCHEME,
  isWindowsDrivePath,
  remarkFilePathLinks,
} from "../filePathLinks";
import { markdownStreamInterval } from "../markdownLinks";
import { recordConversationRenderTrace } from "../platform";
import { useWorkspacePathStatus } from "./WorkspacePathProvider";
import "./MarkdownContent.css";

export type MarkdownContentProps = {
  text: string;
  className?: string;
  streaming?: boolean;
  onOpenLink?(href: string): void;
  renderTrace?: ConversationMarkdownTraceContext;
};

type RemarkPlugins = Options["remarkPlugins"];

const basePlugins: RemarkPlugins = [remarkGfm];
const pathLinkPlugins: RemarkPlugins = [remarkGfm, remarkFilePathLinks];

const MarkdownLinkHandlerContext =
  createContext<MarkdownContentProps["onOpenLink"]>(undefined);

const markdownComponents: Components = {
  a: MarkdownAnchor,
  img: MarkdownImage,
};

export function MarkdownContent({
  text,
  className = "",
  streaming = false,
  onOpenLink,
  renderTrace,
}: MarkdownContentProps) {
  const renderedText = useThrottledMarkdown(text, streaming);
  useConversationMarkdownTrace(renderedText, renderTrace);
  const instanceId = useId().replaceAll(":", "");
  return (
    <MarkdownLinkHandlerContext.Provider value={onOpenLink}>
      <MemoizedMarkdown
        className={`markdown-content ${className}`.trim()}
        clobberPrefix={`opentopia-${instanceId}-`}
        linkifyPaths={Boolean(onOpenLink)}
        text={renderedText}
      />
    </MarkdownLinkHandlerContext.Provider>
  );
}

const MemoizedMarkdown = memo(function MemoizedMarkdown({
  className,
  clobberPrefix,
  linkifyPaths,
  text,
}: {
  className: string;
  clobberPrefix: string;
  linkifyPaths: boolean;
  text: string;
}) {
  return (
    <div className={className}>
      <ReactMarkdown
        components={markdownComponents}
        remarkPlugins={linkifyPaths ? pathLinkPlugins : basePlugins}
        remarkRehypeOptions={{ clobberPrefix }}
        skipHtml
        urlTransform={markdownUrlTransform}
      >
        {text}
      </ReactMarkdown>
    </div>
  );
});

function markdownUrlTransform(url: string): string {
  if (url.startsWith(FILE_PATH_LINK_SCHEME) || isWindowsDrivePath(url)) {
    return url;
  }
  return defaultUrlTransform(url);
}

function MarkdownAnchor({
  href,
  children,
  ...props
}: AnchorHTMLAttributes<HTMLAnchorElement>) {
  const onOpenLink = useContext(MarkdownLinkHandlerContext);
  const linkInfo = href ? decodeFilePathHref(href) : null;
  const pathStatus = useWorkspacePathStatus(linkInfo?.path ?? null);

  // File-detected link (opentopia-file: scheme): render as a compact chip once
  // the file is confirmed to exist, otherwise show plain text.
  if (linkInfo) {
    if (pathStatus !== "known") return <>{children}</>;

    const segments = linkInfo.path.split("/").filter(Boolean);
    const fileName = segments.at(-1) ?? linkInfo.path;
    const lineLabel = linkInfo.fragment
      ? `(line ${linkInfo.fragment.replace(/^L+/, "")})`
      : null;

    return (
      <a
        href={href}
        title={linkInfo.path}
        className="markdown-path-chip"
        onClick={(event) => {
          if (event.defaultPrevented || !href) return;
          if (!onOpenLink) return;
          event.preventDefault();
          onOpenLink(href);
        }}
      >
        <FileText size={12} strokeWidth={2} aria-hidden />
        <span>{fileName}</span>
        {lineLabel && (
          <span className="markdown-path-chip-line">{lineLabel}</span>
        )}
      </a>
    );
  }

  // Normal markdown link
  return (
    <a
      {...props}
      href={href}
      onClick={(event) => {
        props.onClick?.(event);
        if (event.defaultPrevented || !href || href.startsWith("#")) return;
        if (!onOpenLink) return;
        event.preventDefault();
        onOpenLink(href);
      }}
    >
      {children}
    </a>
  );
}

function MarkdownImage({
  src,
  alt,
  ...props
}: ImgHTMLAttributes<HTMLImageElement>) {
  const safeSrc = src ? defaultUrlTransform(src) : "";
  if (!safeSrc) return <span className="markdown-image-alt">{alt}</span>;
  return (
    <img
      {...props}
      alt={alt ?? ""}
      decoding="async"
      loading="lazy"
      referrerPolicy="no-referrer"
      src={safeSrc}
    />
  );
}

function useThrottledMarkdown(text: string, streaming: boolean): string {
  const [renderedText, setRenderedText] = useState(text);
  const latestTextRef = useRef(text);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    latestTextRef.current = text;
    if (!streaming) {
      if (timerRef.current) clearTimeout(timerRef.current);
      timerRef.current = null;
      setRenderedText(text);
      return;
    }
    if (timerRef.current) return;
    timerRef.current = setTimeout(() => {
      timerRef.current = null;
      setRenderedText(latestTextRef.current);
    }, markdownStreamInterval(text.length));
  }, [streaming, text]);

  useEffect(
    () => () => {
      if (timerRef.current) {
        clearTimeout(timerRef.current);
        timerRef.current = null;
      }
    },
    [],
  );

  // The terminal event can arrive before a scheduled streaming paint. Render
  // the authoritative final value immediately instead of showing a stale tail
  // for one more effect cycle.
  return streaming ? renderedText : text;
}

function useConversationMarkdownTrace(
  text: string,
  context: ConversationMarkdownTraceContext | undefined,
) {
  const contextRef = useRef(context);
  const latestTextRef = useRef(text);
  const committedTextRef = useRef("");
  const paintedTextRef = useRef("");
  const paintJobRef = useRef<{
    frame: number | null;
    timer: number | null;
  }>({ frame: null, timer: null });

  contextRef.current = context;
  latestTextRef.current = text;

  useLayoutEffect(() => {
    const activeContext = contextRef.current;
    if (!activeContext) return;
    const change = renderedTextChange(committedTextRef.current, text);
    committedTextRef.current = text;
    if (change) {
      recordConversationRenderTrace({
        ...traceBase(activeContext, "committed"),
        ...change,
        visible: true,
      });
    }

    const job = paintJobRef.current;
    if (job.frame !== null || job.timer !== null) return;
    job.frame = window.requestAnimationFrame(() => {
      job.frame = null;
      job.timer = window.setTimeout(() => {
        job.timer = null;
        const paintedText = latestTextRef.current;
        const paintedChange = renderedTextChange(
          paintedTextRef.current,
          paintedText,
        );
        paintedTextRef.current = paintedText;
        const latestContext = contextRef.current;
        if (!paintedChange || !latestContext) return;
        recordConversationRenderTrace({
          ...traceBase(latestContext, "painted"),
          ...paintedChange,
          visible: true,
        });
      }, 0);
    });
  }, [text]);

  useEffect(
    () => () => {
      const job = paintJobRef.current;
      if (job.frame !== null) {
        window.cancelAnimationFrame(job.frame);
        job.frame = null;
      }
      if (job.timer !== null) {
        window.clearTimeout(job.timer);
        job.timer = null;
      }
    },
    [],
  );
}

function traceBase(
  context: ConversationMarkdownTraceContext,
  stage: ConversationRenderTrace["stage"],
): Omit<
  ConversationRenderTrace,
  "change" | "text" | "textLength" | "visible"
> {
  return {
    stage,
    channel: context.channel,
    threadId: context.threadId,
    turnId: context.turnId,
    messageId: context.messageId,
    ...rendererTraceTime(),
  };
}
