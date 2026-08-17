import {
  Children,
  createContext,
  isValidElement,
  memo,
  useContext,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type AnchorHTMLAttributes,
  type HTMLAttributes,
  type ImgHTMLAttributes,
  type MouseEvent as ReactMouseEvent,
  type ReactNode,
} from "react";
import ReactMarkdown, {
  defaultUrlTransform,
  type Components,
  type Options,
} from "react-markdown";
import remarkGfm from "remark-gfm";
import {
  decodeAttachmentLink,
  ATTACHMENT_LINK_SCHEME,
  remarkAttachmentLinks,
} from "../attachmentLinks";
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
  markdownFileActionPath,
  markdownFileLinkDisplayState,
  markdownStreamInterval,
  resolveMarkdownFileLink,
} from "../markdownLinks";
import { recordConversationRenderTrace } from "../platform";
import type { ContextSourceRef } from "../types";
import { FileLinkContextMenu } from "./FileLinkContextMenu";
import { FileTypeIcon } from "./FileTypeIcon";
import { MermaidDiagram } from "./MermaidDiagram";
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
  attachmentSources?: readonly ContextSourceRef[];
  onOpenAttachment?(source: ContextSourceRef): void;
  renderTrace?: ConversationMarkdownTraceContext;
};

type RemarkPlugins = Options["remarkPlugins"];

type MarkdownLinkContextValue = Pick<
  MarkdownContentProps,
  | "attachmentSources"
  | "baseWorkspacePath"
  | "onOpenAttachment"
  | "onOpenLink"
  | "streaming"
>;

const MarkdownLinkContext = createContext<MarkdownLinkContextValue>({});

const markdownComponents: Components = {
  a: MarkdownAnchor,
  img: MarkdownImage,
  pre: MarkdownPre,
};

export function MarkdownContent({
  text,
  className = "",
  streaming = false,
  baseWorkspacePath,
  onOpenLink,
  attachmentSources = [],
  onOpenAttachment,
  renderTrace,
}: MarkdownContentProps) {
  const renderedText = useThrottledMarkdown(text, streaming);
  useConversationMarkdownTrace(renderedText, renderTrace);
  const instanceId = useId().replaceAll(":", "");
  const linkifyAttachments = Boolean(
    onOpenAttachment && attachmentSources.length,
  );
  return (
    <MarkdownLinkContext.Provider
      value={{
        attachmentSources,
        baseWorkspacePath,
        onOpenAttachment,
        onOpenLink,
        streaming,
      }}
    >
      <MemoizedMarkdown
        attachmentSources={attachmentSources}
        className={`markdown-content ${className}`.trim()}
        clobberPrefix={`opentopia-${instanceId}-`}
        linkifyAttachments={linkifyAttachments}
        linkifyPaths={Boolean(onOpenLink)}
        text={renderedText}
      />
    </MarkdownLinkContext.Provider>
  );
}

const MemoizedMarkdown = memo(function MemoizedMarkdown({
  attachmentSources,
  className,
  clobberPrefix,
  linkifyAttachments,
  linkifyPaths,
  text,
}: {
  attachmentSources: readonly ContextSourceRef[];
  className: string;
  clobberPrefix: string;
  linkifyAttachments: boolean;
  linkifyPaths: boolean;
  text: string;
}) {
  const remarkPlugins: RemarkPlugins = [remarkGfm];
  if (linkifyAttachments) {
    remarkPlugins.push([remarkAttachmentLinks, { sources: attachmentSources }]);
  }
  if (linkifyPaths) remarkPlugins.push(remarkFilePathLinks);
  return (
    <div className={className}>
      <ReactMarkdown
        components={markdownComponents}
        remarkPlugins={remarkPlugins}
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
  if (
    url.startsWith(ATTACHMENT_LINK_SCHEME) ||
    url.startsWith(FILE_PATH_LINK_SCHEME) ||
    isWindowsDrivePath(url)
  ) {
    return url;
  }
  return defaultUrlTransform(url);
}

function MarkdownPre({ children, ...props }: HTMLAttributes<HTMLPreElement>) {
  const { streaming } = useContext(MarkdownLinkContext);
  const mermaidSource = markdownMermaidSource(children);

  // Rendering a still-streaming diagram causes repeated parse failures and
  // expensive redraws. Keep showing its ordinary code block until the turn is
  // complete, then replace it with the finished diagram.
  if (!streaming && mermaidSource !== null) {
    return <MermaidDiagram source={mermaidSource} />;
  }

  return <pre {...props}>{children}</pre>;
}

function markdownMermaidSource(children: ReactNode): string | null {
  const childItems = Children.toArray(children);
  if (childItems.length !== 1) return null;
  const code = childItems[0];
  if (
    !isValidElement<{
      children?: ReactNode;
      className?: string;
    }>(code) ||
    !/(?:^|\s)language-mermaid(?:\s|$)/.test(code.props.className ?? "")
  ) {
    return null;
  }

  const source = markdownCodeText(code.props.children);
  return source?.replace(/\n$/, "") ?? null;
}

function markdownCodeText(value: ReactNode): string | null {
  if (typeof value === "string" || typeof value === "number") {
    return String(value);
  }
  if (!Array.isArray(value)) return null;

  const parts = value.map(markdownCodeText);
  return parts.every((part): part is string => part !== null)
    ? parts.join("")
    : null;
}

function MarkdownAnchor({
  href,
  children,
  ...props
}: AnchorHTMLAttributes<HTMLAnchorElement>) {
  const {
    attachmentSources = [],
    baseWorkspacePath,
    onOpenAttachment,
    onOpenLink,
  } = useContext(MarkdownLinkContext);
  const attachmentId = href ? decodeAttachmentLink(href) : null;
  const attachment = attachmentId
    ? attachmentSources.find((source) => source.id === attachmentId)
    : undefined;
  const detectedLinkInfo = href ? decodeFilePathHref(href) : null;
  const linkInfo = href
    ? resolveMarkdownFileLink(href, baseWorkspacePath)
    : null;
  const workspaceLinkPath =
    linkInfo?.kind === "workspace" ? linkInfo.path : null;
  const workspacePathStatus = useWorkspacePathStatus(workspaceLinkPath);
  const pathStatus = linkInfo?.kind === "local" ? "known" : workspacePathStatus;
  const fileLinkDisplayState = markdownFileLinkDisplayState(
    detectedLinkInfo !== null,
    pathStatus,
  );
  const workspaceAbsolutePath = useWorkspaceAbsolutePath(workspaceLinkPath);
  const absolutePath =
    linkInfo?.kind === "local" ? linkInfo.path : workspaceAbsolutePath;
  const fileActionPath = markdownFileActionPath(linkInfo, absolutePath);
  const readFileText = useWorkspaceFileTextReader(workspaceLinkPath);
  const anchorRef = useRef<HTMLAnchorElement>(null);
  const [contextMenuPoint, setContextMenuPoint] = useState<{
    x: number;
    y: number;
  } | null>(null);
  const line = markdownLineNumber(linkInfo?.fragment ?? null);

  function handleFileContextMenu(event: ReactMouseEvent<HTMLAnchorElement>) {
    props.onContextMenu?.(event);
    if (event.defaultPrevented || !fileActionPath) return;
    event.preventDefault();
    event.stopPropagation();
    const bounds = event.currentTarget.getBoundingClientRect();
    setContextMenuPoint({
      x: event.clientX || bounds.left,
      y: event.clientY || bounds.bottom,
    });
  }

  const fileContextMenu =
    fileActionPath && contextMenuPoint ? (
      <FileLinkContextMenu
        line={line}
        onClose={(options) => {
          setContextMenuPoint(null);
          if (options?.restoreFocus) {
            window.requestAnimationFrame(() => anchorRef.current?.focus());
          }
        }}
        onOpen={
          fileLinkDisplayState === "link" && onOpenLink && href
            ? () => onOpenLink(href)
            : undefined
        }
        path={fileActionPath}
        point={contextMenuPoint}
        readText={
          fileLinkDisplayState === "link"
            ? (readFileText ?? undefined)
            : undefined
        }
      />
    ) : null;

  if (attachment) {
    return (
      <a
        {...props}
        href={href}
        title={attachment.name}
        className={[props.className, "markdown-attachment-link"]
          .filter(Boolean)
          .join(" ")}
        onClick={(event) => {
          props.onClick?.(event);
          if (event.defaultPrevented || !onOpenAttachment) return;
          event.preventDefault();
          onOpenAttachment(attachment);
        }}
      >
        <FileTypeIcon
          name={attachment.name}
          contentType={attachment.contentType}
          size={14}
        />
        {children}
      </a>
    );
  }

  // Keep the visible label stable while the workspace index verifies whether
  // the target exists. Verification controls interactivity, not presentation,
  // so an asynchronous directory lookup never flashes the raw full path.
  if (linkInfo && fileLinkDisplayState !== "fallback") {
    const targetTitle = linkInfo.fragment
      ? `${linkInfo.path}#${linkInfo.fragment}`
      : linkInfo.path;

    if (fileLinkDisplayState === "label") {
      return (
        <span className="markdown-file-link-label" title={targetTitle}>
          {linkInfo.fileName}
        </span>
      );
    }

    return (
      <>
        <a
          aria-haspopup="menu"
          data-text-context-menu="custom"
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
            handleFileContextMenu(event);
          }}
        >
          {linkInfo.fileName}
        </a>
        {fileContextMenu}
      </>
    );
  }

  // Automatically detected prose should not become a broken link when its
  // path is missing. Explicit Markdown links retain their original behavior.
  if (detectedLinkInfo) return <>{children}</>;

  // Normal markdown link
  return (
    <>
      <a
        {...props}
        aria-haspopup={fileActionPath ? "menu" : props["aria-haspopup"]}
        data-text-context-menu={fileActionPath ? "custom" : undefined}
        href={href}
        ref={fileActionPath ? anchorRef : undefined}
        onClick={(event) => {
          props.onClick?.(event);
          if (event.defaultPrevented || !href || href.startsWith("#")) return;
          if (!onOpenLink) return;
          event.preventDefault();
          onOpenLink(href);
        }}
        onContextMenu={
          fileActionPath ? handleFileContextMenu : props.onContextMenu
        }
      >
        {children}
      </a>
      {fileContextMenu}
    </>
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
