import type { AgentEvent } from "./types";

export function isContextCompactionActivityEvent(event: AgentEvent): boolean {
  const payload = event.payload;
  return (
    (payload.type === "model_context_built" &&
      payload.purpose === "context_compaction") ||
    payload.type === "context_compacted" ||
    (payload.type === "context_warning" && payload.stage.includes("compaction"))
  );
}
