import type { FlowRunStatus, WorkflowCheckpointStatus } from "../../types";
import type { BadgeVariant } from "../ui/Badge";
import { humanizeIdentifier } from "./enterpriseSidebarPresentation.ts";
import {
  defaultApplicationLanguage,
  interfaceMessage,
  type ApplicationLanguage,
  type InterfaceMessageKey,
} from "../../applicationLanguage.ts";

export type RunStatusPresentation = {
  description: string;
  label: string;
  variant: BadgeVariant;
};

const RUN_STATUS_PRESENTATION: Record<
  FlowRunStatus,
  {
    description: InterfaceMessageKey;
    label: InterfaceMessageKey;
    variant: BadgeVariant;
  }
> = {
  queued: {
    description: "flow.runStatus.queued.description",
    label: "flow.runStatus.queued.label",
    variant: "neutral",
  },
  running: {
    description: "flow.runStatus.running.description",
    label: "flow.runStatus.running.label",
    variant: "info",
  },
  pause_requested: {
    description: "flow.runStatus.pauseRequested.description",
    label: "flow.runStatus.pauseRequested.label",
    variant: "warning",
  },
  paused: {
    description: "flow.runStatus.paused.description",
    label: "flow.runStatus.paused.label",
    variant: "warning",
  },
  waiting_approval: {
    description: "flow.runStatus.waitingApproval.description",
    label: "flow.runStatus.waitingApproval.label",
    variant: "warning",
  },
  waiting_human: {
    description: "flow.runStatus.waitingHuman.description",
    label: "flow.runStatus.waitingHuman.label",
    variant: "warning",
  },
  resuming: {
    description: "flow.runStatus.resuming.description",
    label: "flow.runStatus.resuming.label",
    variant: "info",
  },
  succeeded: {
    description: "flow.runStatus.succeeded.description",
    label: "flow.runStatus.succeeded.label",
    variant: "success",
  },
  failed: {
    description: "flow.runStatus.failed.description",
    label: "flow.runStatus.failed.label",
    variant: "danger",
  },
  cancel_requested: {
    description: "flow.runStatus.cancelRequested.description",
    label: "flow.runStatus.cancelRequested.label",
    variant: "warning",
  },
  cancelled: {
    description: "flow.runStatus.cancelled.description",
    label: "flow.runStatus.cancelled.label",
    variant: "danger",
  },
};

export type PayloadFieldPresentation = {
  description: string | null;
  key: string;
  label: string;
  schema: unknown;
  value: unknown;
};

export function runStatusPresentation(
  status: FlowRunStatus,
  language: ApplicationLanguage = defaultApplicationLanguage,
): RunStatusPresentation {
  const presentation = RUN_STATUS_PRESENTATION[status];
  return {
    description: interfaceMessage(language, presentation.description),
    label: interfaceMessage(language, presentation.label),
    variant: presentation.variant,
  };
}

export function checkpointStatusLabel(
  status: WorkflowCheckpointStatus,
  language: ApplicationLanguage = defaultApplicationLanguage,
): string {
  if (status === "running")
    return interfaceMessage(language, "flow.checkpoint.saving");
  if (status === "committed")
    return interfaceMessage(language, "flow.checkpoint.saved");
  if (status === "failed")
    return interfaceMessage(language, "flow.checkpoint.failed");
  return interfaceMessage(language, "flow.checkpoint.cancelled");
}

export function payloadFields(
  payload: Record<string, unknown>,
  schema: unknown,
  language: ApplicationLanguage = defaultApplicationLanguage,
): PayloadFieldPresentation[] {
  const properties = schemaProperties(schema);
  const schemaKeys = Object.keys(properties).filter((key) =>
    Object.hasOwn(payload, key),
  );
  const extraKeys = Object.keys(payload).filter(
    (key) => !Object.hasOwn(properties, key),
  );

  return [...schemaKeys, ...extraKeys].map((key) => {
    const fieldSchema = properties[key];
    return {
      description: schemaText(fieldSchema, "description"),
      key,
      label:
        schemaText(fieldSchema, "title") ??
        platformPayloadFieldLabel(key, language) ??
        humanizeIdentifier(key),
      schema: fieldSchema,
      value: payload[key],
    };
  });
}

const PLATFORM_PAYLOAD_FIELD_LABELS: Record<string, InterfaceMessageKey> = {
  caseid: "flow.payload.field.caseId",
  eventkind: "flow.payload.field.eventKind",
  payloadref: "flow.payload.field.payloadRef",
  summary: "flow.payload.field.summary",
  synthetic: "flow.payload.field.synthetic",
};

function platformPayloadFieldLabel(
  key: string,
  language: ApplicationLanguage,
): string | null {
  const normalized = key.replace(/[^a-z0-9]/gi, "").toLocaleLowerCase();
  const messageKey = PLATFORM_PAYLOAD_FIELD_LABELS[normalized];
  return messageKey ? interfaceMessage(language, messageKey) : null;
}

export function payloadItemSchema(schema: unknown): unknown {
  return isRecord(schema) ? schema.items : undefined;
}

export function formatDuration(
  startedAt: string | null,
  endedAt: string | null,
  language: ApplicationLanguage = defaultApplicationLanguage,
): string {
  if (!startedAt || !endedAt) return "—";
  const start = new Date(startedAt).getTime();
  const end = new Date(endedAt).getTime();
  if (!Number.isFinite(start) || !Number.isFinite(end) || end < start) {
    return "—";
  }

  const totalSeconds = Math.round((end - start) / 1_000);
  if (totalSeconds < 1)
    return interfaceMessage(language, "flow.duration.lessThanSecond");
  if (totalSeconds < 60)
    return `${totalSeconds} ${interfaceMessage(language, "flow.duration.second")}`;

  const hours = Math.floor(totalSeconds / 3_600);
  const minutes = Math.floor((totalSeconds % 3_600) / 60);
  const seconds = totalSeconds % 60;
  if (hours > 0) {
    const hourLabel = interfaceMessage(language, "flow.duration.hour");
    const minuteLabel = interfaceMessage(language, "flow.duration.minute");
    return minutes > 0
      ? `${hours} ${hourLabel} ${minutes} ${minuteLabel}`
      : `${hours} ${hourLabel}`;
  }
  return seconds > 0
    ? `${minutes} ${interfaceMessage(language, "flow.duration.shortMinute")} ${seconds} ${interfaceMessage(language, "flow.duration.second")}`
    : `${minutes} ${interfaceMessage(language, "flow.duration.minute")}`;
}

export function formatDateTime(
  value: string | null,
  language: ApplicationLanguage = defaultApplicationLanguage,
): string {
  if (!value) return "—";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "—";
  return new Intl.DateTimeFormat(language, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  }).format(date);
}

export function formatScalarValue(
  value: unknown,
  language: ApplicationLanguage = defaultApplicationLanguage,
): string | null {
  if (value === null) return "—";
  if (typeof value === "boolean") {
    return interfaceMessage(
      language,
      value ? "flow.value.yes" : "flow.value.no",
    );
  }
  if (typeof value === "string" || typeof value === "number") {
    return String(value);
  }
  return null;
}

function schemaProperties(schema: unknown): Record<string, unknown> {
  if (!isRecord(schema) || !isRecord(schema.properties)) return {};
  return schema.properties;
}

function schemaText(
  schema: unknown,
  key: "description" | "title",
): string | null {
  if (!isRecord(schema)) return null;
  const value = schema[key];
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
