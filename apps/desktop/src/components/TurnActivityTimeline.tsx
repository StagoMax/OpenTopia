import { useEffect, useId, useMemo, useState } from "react";
import { ChevronDown, ChevronRight } from "lucide-react";
import type { AgentEvent } from "../types";
import { shouldShowRecordedTurnChanges } from "../turnChangeOwnership";
import type { ActiveTurnPhase } from "../turnActivityStatus";
import { ShimmerText } from "./ui";
import { ActivityEntryView } from "./turnActivityTimeline/activityGroups";
import {
  activityEntryIsRunning,
  activityEntryKey,
  activityState,
  buildActivityEntries,
  type ActivityState,
} from "./turnActivityTimeline/model";
import {
  useStatusPaintTrace,
  useTimelineClock,
} from "./turnActivityTimeline/hooks";
import {
  formatTurnTiming,
  resolveTurnTimingRange,
  type TurnTimingRange,
} from "./turnActivityTimeline/timing";
export { TurnChangeCard } from "./turnActivityTimeline/TurnChangeCard";
import "./TurnActivityTimeline.css";

export function TurnActivityTimeline({
  events,
  isActive,
  standalone = false,
  formatError = (message) => message,
  onOpenMarkdownLink,
}: {
  events: AgentEvent[];
  isActive: boolean;
  standalone?: boolean;
  formatError?(message: string): string;
  onOpenMarkdownLink?(href: string): void;
}) {
  const entries = useMemo(
    () =>
      buildActivityEntries(events).filter(
        (entry) => entry.kind !== "reasoning",
      ),
    [events],
  );
  const state = useMemo(
    () => activityState(events, isActive),
    [events, isActive],
  );
  const [expanded, setExpanded] = useState(isActive);
  const bodyId = useId();
  const mountedAt = useMemo(() => Date.now(), []);
  const shouldTick =
    isActive || (state === "running" && entries.some(activityEntryIsRunning));
  const turnTimingRange = useMemo(
    () => resolveTurnTimingRange(events, isActive, mountedAt),
    [events, isActive, mountedAt],
  );
  const statusLabel = isActive ? "处理中" : activityStateLabel(state);
  const traceEvent = events.at(-1);
  useStatusPaintTrace(
    isActive ? statusLabel : null,
    traceEvent?.threadId,
    traceEvent?.turnId,
  );

  useEffect(() => {
    if (isActive || state === "error" || state === "waiting") {
      setExpanded(true);
    } else {
      setExpanded(false);
    }
  }, [isActive, state]);

  const { changeSet, hasTurnLifecycle } = useMemo(() => {
    const changeSetEvent = [...events]
      .reverse()
      .find((event) => event.payload.type === "turn_changes_recorded");
    const resolvedChangeSet =
      changeSetEvent?.payload.type === "turn_changes_recorded" &&
      changeSetEvent.turnId &&
      shouldShowRecordedTurnChanges(events, changeSetEvent.turnId)
        ? changeSetEvent.payload.change_set
        : null;
    return {
      changeSet: resolvedChangeSet,
      hasTurnLifecycle: events.some((event) =>
        [
          "turn_started",
          "turn_finished",
          "turn_cancelled",
          "turn_suspended",
          "error",
        ].includes(event.payload.type),
      ),
    };
  }, [events]);
  if (!isActive && entries.length === 0 && !changeSet && !hasTurnLifecycle) {
    return null;
  }
  const entryViews = entries.map((entry) => (
    <ActivityEntryView
      key={activityEntryKey(entry)}
      entry={entry}
      isActive={isActive}
      traceThreadId={traceEvent?.threadId}
      traceTurnId={traceEvent?.turnId}
      formatError={formatError}
      onOpenMarkdownLink={onOpenMarkdownLink}
    />
  ));

  if (standalone) {
    return (
      <section
        className="turn-activity is-standalone"
        data-state={state}
        aria-label="上下文活动"
      >
        <div
          className="turn-activity-body"
          aria-live={isActive ? "polite" : undefined}
        >
          {entryViews}
        </div>
      </section>
    );
  }

  return (
    <section className="turn-activity" data-state={state}>
      <button
        className="turn-activity-header"
        type="button"
        aria-expanded={expanded}
        aria-controls={bodyId}
        onClick={() => setExpanded((current) => !current)}
      >
        <span
          className="turn-activity-heading"
          aria-live={isActive ? "polite" : undefined}
        >
          {isActive ? (
            <strong>
              <ShimmerText>{statusLabel}</ShimmerText>
            </strong>
          ) : (
            <strong>{statusLabel}</strong>
          )}
          <TurnTimingText range={turnTimingRange} shouldTick={shouldTick} />
        </span>
        <span className="turn-activity-chevron" aria-hidden="true">
          {expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
        </span>
      </button>

      {expanded && (
        <div
          className="turn-activity-body"
          id={bodyId}
          aria-live={isActive ? "polite" : undefined}
        >
          {entryViews}
        </div>
      )}
    </section>
  );
}

function TurnTimingText({
  range,
  shouldTick,
}: {
  range: TurnTimingRange | null;
  shouldTick: boolean;
}) {
  const now = useTimelineClock(shouldTick);
  const timing = formatTurnTiming(range, now);
  return timing ? <small>{timing}</small> : null;
}

export function PendingTurnStatus({
  phase,
  threadId,
  turnId,
}: {
  phase: ActiveTurnPhase;
  threadId: string;
  turnId?: string | null;
}) {
  const label = phase === "thinking" ? "正在思考" : "处理中";
  useStatusPaintTrace(label, threadId, turnId);
  return (
    <section className="turn-activity" data-state="running">
      <div
        className="turn-activity-header is-static"
        role="status"
        aria-live="polite"
      >
        <span className="turn-activity-heading">
          <strong>
            <ShimmerText>{label}</ShimmerText>
          </strong>
        </span>
      </div>
    </section>
  );
}

function activityStateLabel(state: ActivityState) {
  if (state === "error") return "执行失败";
  if (state === "cancelled") return "已取消";
  if (state === "waiting") return "等待继续";
  return "已处理";
}
