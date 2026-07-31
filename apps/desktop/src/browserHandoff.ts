import type { AgentEvent } from "./types";

export type BrowserHandoff = Extract<
  AgentEvent["payload"],
  { type: "browser_handoff_required" }
>;

export function activeBrowserHandoff(
  events: AgentEvent[],
  threadId: string | null,
): BrowserHandoff | null {
  if (!threadId) return null;

  for (let index = events.length - 1; index >= 0; index -= 1) {
    const event = events[index];
    if (event.threadId !== threadId) continue;
    if (event.payload.type === "browser_handoff_completed") return null;
    if (event.payload.type === "browser_handoff_required") {
      return event.payload;
    }
  }
  return null;
}
