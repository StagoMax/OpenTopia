import {
  createContext,
  memo,
  startTransition,
  useContext,
  useEffect,
  useId,
  useRef,
  useState,
  type AnchorHTMLAttributes,
  type ImgHTMLAttributes,
} from "react";
import ReactMarkdown, {
  defaultUrlTransform,
  type Components,
} from "react-markdown";
import remarkGfm from "remark-gfm";
import { markdownStreamInterval } from "../markdownLinks";
import "./MarkdownContent.css";

export type MarkdownContentProps = {
  text: string;
  className?: string;
  streaming?: boolean;
  onOpenLink?(href: string): void;
};

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
}: MarkdownContentProps) {
  const renderedText = useThrottledMarkdown(text, streaming);
  const instanceId = useId().replaceAll(":", "");
  return (
    <MarkdownLinkHandlerContext.Provider value={onOpenLink}>
      <MemoizedMarkdown
        className={`markdown-content ${className}`.trim()}
        clobberPrefix={`opentopia-${instanceId}-`}
        text={renderedText}
      />
    </MarkdownLinkHandlerContext.Provider>
  );
}

const MemoizedMarkdown = memo(function MemoizedMarkdown({
  className,
  clobberPrefix,
  text,
}: {
  className: string;
  clobberPrefix: string;
  text: string;
}) {
  return (
    <div className={className}>
      <ReactMarkdown
        components={markdownComponents}
        remarkPlugins={[remarkGfm]}
        remarkRehypeOptions={{ clobberPrefix }}
        skipHtml
      >
        {text}
      </ReactMarkdown>
    </div>
  );
});

function MarkdownAnchor({
  href,
  children,
  ...props
}: AnchorHTMLAttributes<HTMLAnchorElement>) {
  const onOpenLink = useContext(MarkdownLinkHandlerContext);
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
      startTransition(() => setRenderedText(latestTextRef.current));
    }, markdownStreamInterval(text.length));
  }, [streaming, text]);

  useEffect(
    () => () => {
      if (timerRef.current) clearTimeout(timerRef.current);
    },
    [],
  );

  return renderedText;
}
