import { useEffect, useRef, useState } from "react";
import {
  rendererTraceTime,
  type ConversationRenderTrace,
} from "../../conversationRenderTrace";
import { recordConversationRenderTrace } from "../../platform";

export function useTimelineClock(shouldTick: boolean) {
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    setNow(Date.now());
    if (!shouldTick) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1_000);
    return () => window.clearInterval(timer);
  }, [shouldTick]);

  return now;
}

export function useStatusPaintTrace(
  label: string | null,
  threadId: string | undefined,
  turnId: string | null | undefined,
) {
  const paintedLabelRef = useRef("");

  useEffect(() => {
    if (!label || !threadId || paintedLabelRef.current === label) return;
    let timer: number | null = null;
    const frame = window.requestAnimationFrame(() => {
      timer = window.setTimeout(() => {
        paintedLabelRef.current = label;
        const trace: ConversationRenderTrace = {
          stage: "painted",
          channel: "status",
          threadId,
          turnId,
          ...rendererTraceTime(),
          change: "replace",
          text: label,
          textLength: label.length,
          visible: true,
        };
        recordConversationRenderTrace(trace);
      }, 0);
    });
    return () => {
      window.cancelAnimationFrame(frame);
      if (timer !== null) window.clearTimeout(timer);
    };
  }, [label, threadId, turnId]);
}
