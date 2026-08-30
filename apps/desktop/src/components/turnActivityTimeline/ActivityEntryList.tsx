import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type FocusEvent,
} from "react";
import type { ToolResult } from "../../types";
import { ActivityEntryView } from "./activityGroups";
import {
  activityVirtualizationThreshold,
  buildActivityVirtualChunks,
  type ActivityVirtualChunk,
} from "./activityVirtualization";
import { activityEntryKey, type ActivityEntry } from "./model";

type ActivityEntryListProps = {
  entries: ActivityEntry[];
  isActive: boolean;
  traceThreadId?: string;
  traceTurnId?: string | null;
  formatError(message: string): string;
  onOpenMarkdownLink?(href: string): void;
  onLoadToolResultDetail?(eventId: string): Promise<ToolResult>;
};

export function ActivityEntryList(props: ActivityEntryListProps) {
  const { entries, isActive, ...entryProps } = props;
  const chunks = useMemo(() => buildActivityVirtualChunks(entries), [entries]);
  const measuredHeights = useRef(new Map<string, number>());

  if (entries.length <= activityVirtualizationThreshold) {
    return entries.map((entry) => (
      <ActivityEntry
        key={activityEntryKey(entry)}
        entry={entry}
        isActive={isActive}
        {...entryProps}
      />
    ));
  }

  return chunks.map((chunk, index) => (
    <VirtualActivityChunk
      key={chunk.key}
      chunk={chunk}
      initiallyMounted={index === 0 || index >= chunks.length - 2}
      keepMounted={isActive && index === chunks.length - 1}
      measuredHeights={measuredHeights.current}
      isActive={isActive}
      {...entryProps}
    />
  ));
}

function VirtualActivityChunk({
  chunk,
  initiallyMounted,
  keepMounted,
  measuredHeights,
  ...entryProps
}: Omit<ActivityEntryListProps, "entries"> & {
  chunk: ActivityVirtualChunk;
  initiallyMounted: boolean;
  keepMounted: boolean;
  measuredHeights: Map<string, number>;
}) {
  const elementRef = useRef<HTMLDivElement>(null);
  const focusedRef = useRef(false);
  const intersectingRef = useRef(false);
  const [mounted, setMounted] = useState(initiallyMounted || keepMounted);
  const [height, setHeight] = useState(
    () => measuredHeights.get(chunk.key) ?? chunk.estimatedHeight,
  );

  useEffect(() => {
    if (keepMounted) setMounted(true);
  }, [keepMounted]);

  useEffect(() => {
    const element = elementRef.current;
    if (!element || typeof IntersectionObserver === "undefined") {
      setMounted(true);
      return;
    }
    const root = element.closest<HTMLElement>(".message-list");
    const observer = new IntersectionObserver(
      ([entry]) => {
        intersectingRef.current = entry.isIntersecting;
        setMounted(entry.isIntersecting || keepMounted || focusedRef.current);
      },
      { root, rootMargin: "800px 0px" },
    );
    observer.observe(element);
    return () => observer.disconnect();
  }, [keepMounted]);

  useEffect(() => {
    const element = elementRef.current;
    if (!element || !mounted || typeof ResizeObserver === "undefined") return;
    const measure = () => {
      const nextHeight = element.getBoundingClientRect().height;
      if (nextHeight <= 0) return;
      measuredHeights.set(chunk.key, nextHeight);
      setHeight(nextHeight);
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [chunk.key, measuredHeights, mounted]);

  const handleFocus = useCallback(() => {
    focusedRef.current = true;
    setMounted(true);
  }, []);
  const handleBlur = useCallback((event: FocusEvent<HTMLDivElement>) => {
    if (event.currentTarget.contains(event.relatedTarget)) return;
    focusedRef.current = false;
    if (!intersectingRef.current && !keepMounted) setMounted(false);
  }, [keepMounted]);

  return (
    <div
      ref={elementRef}
      className="turn-activity-virtual-chunk"
      data-mounted={mounted || undefined}
      style={mounted ? undefined : { minHeight: height }}
      onFocusCapture={handleFocus}
      onBlurCapture={handleBlur}
    >
      {mounted
        ? chunk.entries.map((entry) => (
            <ActivityEntry
              key={activityEntryKey(entry)}
              entry={entry}
              {...entryProps}
            />
          ))
        : null}
    </div>
  );
}

function ActivityEntry({
  entry,
  ...props
}: Omit<ActivityEntryListProps, "entries"> & { entry: ActivityEntry }) {
  return <ActivityEntryView entry={entry} {...props} />;
}
