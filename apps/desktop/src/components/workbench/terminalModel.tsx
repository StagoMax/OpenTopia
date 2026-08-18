import { ExternalLink } from "lucide-react";
import type { AgentEvent, Message, TerminalEvent } from "../../types";
import { formatBytes, formatTime } from "./workbenchFormat";

export type TerminalRow = {
  id: string;
  kind: "info" | "command" | "output" | "error";
  label: string;
  time: string;
  body?: string;
  artifacts: ArtifactReference[];
  sortKey?: number;
};

type ArtifactReference = {
  id: string;
  kind?: string;
  bytes?: number;
};

export function ArtifactReferenceList({
  artifacts,
  threadId,
  onOpenArtifact,
}: {
  artifacts: ArtifactReference[];
  threadId: string;
  onOpenArtifact(threadId: string, artifactId: string): void;
}) {
  return (
    <div className="artifact-reference-list">
      {artifacts.map((artifact) => (
        <button
          className="artifact-reference-button"
          key={artifact.id}
          type="button"
          title={artifact.id}
          onClick={() => onOpenArtifact(threadId, artifact.id)}
        >
          <ExternalLink size={12} />
          <span>{artifact.kind ?? "artifact"}</span>
          {artifact.bytes !== undefined && (
            <small>{formatBytes(artifact.bytes)}</small>
          )}
        </button>
      ))}
    </div>
  );
}

export function buildCombinedTerminalRows(
  events: AgentEvent[],
  terminalEvents: TerminalEvent[],
): TerminalRow[] {
  const agentTimes = new Map(
    events.map((event) => [event.id, Date.parse(event.createdAt)]),
  );
  const agentRows = buildTerminalRows(events).map((row) => ({
    ...row,
    sortKey: agentTimes.get(row.id) ?? 0,
  }));
  const terminalRows = buildTerminalEventRows(terminalEvents);
  return [...agentRows, ...terminalRows].sort(
    (left, right) => (left.sortKey ?? 0) - (right.sortKey ?? 0),
  );
}

function buildTerminalEventRows(events: TerminalEvent[]): TerminalRow[] {
  return events.map((event) => {
    const time = formatTime(event.createdAt);
    const sortKey = Date.parse(event.createdAt);
    const base = {
      id: event.id,
      time,
      sortKey,
      artifacts: [],
    };

    switch (event.type) {
      case "started":
        return {
          ...base,
          kind: "command",
          label: `$ ${event.command ?? "terminal command"}`,
          body: event.cwd ? `cwd: ${event.cwd}` : undefined,
        };
      case "stdout":
        return {
          ...base,
          kind: "output",
          label: "terminal stdout",
          body: truncateTerminalOutput(event.data ?? ""),
        };
      case "stderr":
        return {
          ...base,
          kind: "error",
          label: "terminal stderr",
          body: truncateTerminalOutput(event.data ?? ""),
        };
      case "finished":
        return {
          ...base,
          kind: event.success ? "info" : "error",
          label: event.success ? "terminal finished" : "terminal exited",
          body: terminalExitBody(event),
        };
      case "cancelled":
        return {
          ...base,
          kind: "error",
          label: "terminal cancelled",
          body: event.message ?? "command cancelled",
        };
      case "error":
        return {
          ...base,
          kind: "error",
          label: "terminal error",
          body: event.message ?? "terminal error",
        };
    }
  });
}

function terminalExitBody(event: TerminalEvent): string | undefined {
  const parts = [
    event.success === undefined || event.success === null
      ? undefined
      : event.success
        ? "成功"
        : "失败",
    event.message ?? undefined,
  ].filter(Boolean);
  return parts.length ? parts.join("\n") : undefined;
}

function buildTerminalRows(events: AgentEvent[]): TerminalRow[] {
  return events
    .filter((event) => event.payload.type !== "model_delta")
    .map((event) => {
      const time = formatTime(event.createdAt);
      switch (event.payload.type) {
        case "turn_started":
          return {
            id: event.id,
            kind: "info",
            label: "turn started",
            time,
            body: event.payload.user_message_id,
            artifacts: [],
          };
        case "tool_call_started":
          return {
            id: event.id,
            kind: "command",
            label: `$ ${event.payload.call.name}`,
            time,
            body: formatUnknown(event.payload.call.input),
            artifacts: [],
          };
        case "tool_call_finished":
          return {
            id: event.id,
            kind: "output",
            label: "tool output",
            time,
            body: truncateTerminalOutput(event.payload.result.output),
            artifacts: collectArtifactReferences(
              event.payload.result.metadata,
              event.payload.result.output,
            ),
          };
        case "work_form_updated":
          return {
            id: event.id,
            kind: "info",
            label: "work form updated",
            time,
            body: event.payload.form.items
              .map((item) => `[${item.status}] ${item.title || item.id}`)
              .join("\n"),
            artifacts: [],
          };
        case "assistant_message":
          return {
            id: event.id,
            kind: "info",
            label: "assistant message",
            time,
            artifacts: collectMessageArtifactReferences(event.payload.message),
          };
        case "file_changed":
          return {
            id: event.id,
            kind: "info",
            label: `file changed: ${event.payload.path}`,
            time,
            body: event.payload.summary,
            artifacts: [],
          };
        case "approval_requested":
          return {
            id: event.id,
            kind: "command",
            label: "approval requested",
            time,
            body: `${event.payload.action}\n\n${event.payload.reason}`,
            artifacts: [],
          };
        case "context_compacted":
          return {
            id: event.id,
            kind: "info",
            label: "context compacted",
            time,
            body: event.payload.summary.summary,
            artifacts: [],
          };
        case "context_projection_built":
          return {
            id: event.id,
            kind: "info",
            label: "context projection built",
            time,
            body: `${event.payload.projection.checkpointTokens} checkpoint tokens, ${event.payload.projection.recentTailTokens} recent-tail tokens`,
            artifacts: [],
          };
        case "provider_context_state_updated":
          return {
            id: event.id,
            kind: "info",
            label: "provider context updated",
            time,
            body: `${event.payload.state_kind.replaceAll("_", " ")}, ${event.payload.response_item_count} items`,
            artifacts: [],
          };
        case "provider_context_state_invalidated":
          return {
            id: event.id,
            kind: "info",
            label: "provider context rebuilt",
            time,
            body: event.payload.reason,
            artifacts: [],
          };
        case "turn_finished":
          return {
            id: event.id,
            kind: "info",
            label: "turn finished",
            time,
            body: event.payload.summary,
            artifacts: [],
          };
        case "error":
          return {
            id: event.id,
            kind: "error",
            label: "agent error",
            time,
            body: event.payload.message,
            artifacts: [],
          };
      }
    })
    .filter((row): row is TerminalRow => row !== undefined);
}

function collectMessageArtifactReferences(
  message: Message,
): ArtifactReference[] {
  const refs: ArtifactReference[] = [];
  for (const part of message.parts) {
    if (part.type === "text") {
      refs.push(...artifactReferencesFromText(part.text));
    } else if (part.type === "tool_result") {
      refs.push(
        ...collectArtifactReferences(part.result.metadata, part.result.output),
      );
    }
  }
  return uniqueArtifactReferences(refs);
}

function collectArtifactReferences(
  metadata: unknown,
  output?: string,
): ArtifactReference[] {
  return uniqueArtifactReferences([
    ...artifactReferencesFromMetadata(metadata),
    ...artifactReferencesFromText(output ?? ""),
  ]);
}

function artifactReferencesFromMetadata(
  metadata: unknown,
): ArtifactReference[] {
  if (!isRecord(metadata)) return [];
  const refs: ArtifactReference[] = [];
  const artifactId = readString(metadata.artifactId);
  if (artifactId) {
    refs.push({
      id: artifactId,
      kind: readString(metadata.artifactKind),
      bytes: readNumber(metadata.artifactBytes),
    });
  }
  if (isRecord(metadata.artifact)) {
    const nestedId = readString(metadata.artifact.id);
    if (nestedId) {
      refs.push({
        id: nestedId,
        kind: readString(metadata.artifact.kind),
        bytes: readNumber(metadata.artifact.bytes),
      });
    }
  }
  return refs;
}

function artifactReferencesFromText(text: string): ArtifactReference[] {
  const refs: ArtifactReference[] = [];
  const pattern =
    /\[Artifact:\s*([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12})\]/g;
  let match: RegExpExecArray | null;
  while ((match = pattern.exec(text)) !== null) {
    refs.push({ id: match[1] });
  }
  return refs;
}

function uniqueArtifactReferences(
  refs: ArtifactReference[],
): ArtifactReference[] {
  const byId = new Map<string, ArtifactReference>();
  for (const ref of refs) {
    byId.set(ref.id, { ...byId.get(ref.id), ...ref });
  }
  return [...byId.values()];
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function readString(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value : undefined;
}

function readNumber(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value)
    ? value
    : undefined;
}

function formatUnknown(value: unknown): string {
  if (value === undefined || value === null) return "";
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

function truncateTerminalOutput(output: string): string {
  const limit = 12000;
  if (output.length <= limit) return output;
  return `${output.slice(0, limit)}\n\n[output truncated in UI]`;
}
