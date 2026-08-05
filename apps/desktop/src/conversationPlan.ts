import type { AgentEvent, TaskPlan } from "./types";

const terminalRuntimePlanEvents = new Set([
  "turn_finished",
  "turn_cancelled",
  "error",
]);

export function resolveRuntimeTaskPlan(events: AgentEvent[]): TaskPlan | null {
  const latestPlanEvent = [...events]
    .sort((left, right) => right.seq - left.seq)
    .find((event) => event.payload.type === "plan_updated");
  if (latestPlanEvent?.payload.type !== "plan_updated") return null;

  const ended = Boolean(
    latestPlanEvent.turnId &&
      events.some(
        (event) =>
          event.seq > latestPlanEvent.seq &&
          event.turnId === latestPlanEvent.turnId &&
          terminalRuntimePlanEvents.has(event.payload.type),
      ),
  );
  return ended ? null : latestPlanEvent.payload.plan;
}
