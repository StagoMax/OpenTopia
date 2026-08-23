import type { AgentEvent, GoalSnapshot, WorkForm } from "./types";

const terminalRuntimeFormEvents = new Set([
  "turn_finished",
  "turn_cancelled",
  "error",
]);

export function resolveRuntimeWorkForm(
  events: AgentEvent[],
  activeTurnId?: string | null,
): WorkForm | null {
  const latestFormEvent = [...events]
    .sort((left, right) => right.seq - left.seq)
    .find((event) => event.payload.type === "work_form_updated");
  if (latestFormEvent?.payload.type !== "work_form_updated") return null;

  // The session status is authoritative. Cancellation can be reconciled by a
  // status read before the terminal event reaches the event stream, so relying
  // on events alone leaves a stale, apparently running plan above the composer.
  if (activeTurnId === null) return null;
  if (
    activeTurnId !== undefined &&
    latestFormEvent.turnId &&
    latestFormEvent.turnId !== activeTurnId
  ) {
    return null;
  }

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
  activeTurnId?: string | null,
): WorkForm | null {
  const latestRuntimeForm = resolveRuntimeWorkForm(events, activeTurnId);
  const goalForm = snapshot?.workForm ?? null;
  if (
    goalForm &&
    (!latestRuntimeForm || latestRuntimeForm.id === goalForm.id)
  ) {
    return goalForm;
  }
  return latestRuntimeForm ?? goalForm;
}
