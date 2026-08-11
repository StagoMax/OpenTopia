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
import {
  markdownStreamInterval,
  resolveMarkdownFileLink,
} from "../markdownLinks";
import { recordConversationRenderTrace } from "../platform";
import { FileLinkContextMenu } from "./FileLinkContextMenu";
import {
  useWorkspaceAbsolutePath,
  useWorkspaceFileTextReader,
  useWorkspacePathStatus,
} from "./WorkspacePathProvider";
import "./MarkdownContent.css";

export type MarkdownContentProps = {
  text: string;
  className?: string;
  streaming?: boolean;
  baseWorkspacePath?: string | null;
  onOpenLink?(href: string): void;
  renderTrace?: ConversationMarkdownTraceContext;
};

type RemarkPlugins = Options["remarkPlugins"];

const basePlugins: RemarkPlugins = [remarkGfm];
const pathLinkPlugins: RemarkPlugins = [remarkGfm, remarkFilePathLinks];

type MarkdownLinkContextValue = Pick<
  MarkdownContentProps,
  "baseWorkspacePath" | "onOpenLink"
>;

const MarkdownLinkContext = createContext<MarkdownLinkContextValue>({});

const markdownComponents: Components = {
  a: MarkdownAnchor,
  img: MarkdownImage,
};

export function MarkdownContent({
  text,
  className = "",
  streaming = false,
  baseWorkspacePath,
  onOpenLink,
  renderTrace,
}: MarkdownContentProps) {
  const renderedText = useThrottledMarkdown(text, streaming);
  useConversationMarkdownTrace(renderedText, renderTrace);
  const instanceId = useId().replaceAll(":", "");
  return (
    <MarkdownLinkContext.Provider value={{ baseWorkspacePath, onOpenLink }}>
      <MemoizedMarkdown
        className={`markdown-content ${className}`.trim()}
        clobberPrefix={`opentopia-${instanceId}-`}
        linkifyPaths={Boolean(onOpenLink)}
        text={renderedText}
      />
    </MarkdownLinkContext.Provider>
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
  const { baseWorkspacePath, onOpenLink } = useContext(MarkdownLinkContext);
  const detectedLinkInfo = href ? decodeFilePathHref(href) : null;
  const linkInfo = href
    ? resolveMarkdownFileLink(href, baseWorkspacePath)
    : null;
  const pathStatus = useWorkspacePathStatus(linkInfo?.path ?? null);
  const absolutePath = useWorkspaceAbsolutePath(linkInfo?.path ?? null);
  const readFileText = useWorkspaceFileTextReader(linkInfo?.path ?? null);
  const anchorRef = useRef<HTMLAnchorElement>(null);
  const [contextMenuPoint, setContextMenuPoint] = useState<{
    x: number;
    y: number;
  } | null>(null);

  // Both automatically detected paths and explicit Markdown file links use a
  // filename-only label after the target is confirmed in the workspace.
  if (linkInfo && pathStatus === "known") {
    const targetTitle = linkInfo.fragment
      ? `${linkInfo.path}#${linkInfo.fragment}`
      : linkInfo.path;

    const line = markdownLineNumber(linkInfo.fragment);

    return (
      <>
        <a
          aria-haspopup="menu"
          href={href}
          title={targetTitle}
          className="markdown-file-link"
          ref={anchorRef}
          onClick={(event) => {
            if (event.defaultPrevented || !href) return;
            if (!onOpenLink) return;
            event.preventDefault();
            onOpenLink(href);
          }}
          onContextMenu={(event) => {
            if (!absolutePath) return;
            event.preventDefault();
            event.stopPropagation();
            const bounds = event.currentTarget.getBoundingClientRect();
            setContextMenuPoint({
              x: event.clientX || bounds.left,
              y: event.clientY || bounds.bottom,
            });
          }}
        >
          {linkInfo.fileName}
        </a>
        {absolutePath && contextMenuPoint ? (
          <FileLinkContextMenu
            line={line}
            onClose={(options) => {
              setContextMenuPoint(null);
              if (options?.restoreFocus) {
                window.requestAnimationFrame(() => anchorRef.current?.focus());
              }
            }}
            onOpen={onOpenLink && href ? () => onOpenLink(href) : undefined}
            path={absolutePath}
            point={contextMenuPoint}
            readText={readFileText ?? undefined}
          />
        ) : null}
      </>
    );
  }

  // Automatically detected prose should not become a broken link when its
  // path is missing. Explicit Markdown links retain their original behavior.
  if (detectedLinkInfo) return <>{children}</>;

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

function markdownLineNumber(fragment: string | null): number | null {
  const value = /^L(\d+)(?:C\d+)?$/i.exec(fragment ?? "")?.[1];
  if (!value) return null;
  const line = Number.parseInt(value, 10);
  return Number.isSafeInteger(line) && line > 0 ? line : null;
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
): Omit<ConversationRenderTrace, "change" | "text" | "textLength" | "visible"> {
  return {
    stage,
    channel: context.channel,
    threadId: context.threadId,
    turnId: context.turnId,
    messageId: context.messageId,
    ...rendererTraceTime(),
  };
}
