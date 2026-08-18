import type { AgentEvent, GoalSnapshot, WorkForm } from "./types";

const terminalRuntimeFormEvents = new Set([
  "turn_finished",
  "turn_cancelled",
  "error",
]);

export function resolveRuntimeWorkForm(events: AgentEvent[]): WorkForm | null {
  const latestFormEvent = [...events]
    .sort((left, right) => right.seq - left.seq)
    .find((event) => event.payload.type === "work_form_updated");
  if (latestFormEvent?.payload.type !== "work_form_updated") return null;

  const ended = Boolean(
    latestFormEvent.turnId &&
    events.some(
      (event) =>
        event.seq > latestFormEvent.seq &&
        event.turnId === latestFormEvent.turnId &&
        terminalRuntimeFormEvents.has(event.payload.type),
    ),
  );
  return ended ? null : latestFormEvent.payload.form;
}

export function resolveComposerWorkForm(
  events: AgentEvent[],
  snapshot: GoalSnapshot | null,
): WorkForm | null {
  const latestRuntimeForm = resolveRuntimeWorkForm(events);
  const goalForm = snapshot?.workForm ?? null;
  if (
    goalForm &&
    (!latestRuntimeForm || latestRuntimeForm.id === goalForm.id)
  ) {
    return goalForm;
  }
  return latestRuntimeForm ?? goalForm;
}
