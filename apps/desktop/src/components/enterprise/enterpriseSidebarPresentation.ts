import type { FlowCase, WorkflowTrigger } from "../../types";

const preferredInputKeys = [
  "title",
  "subject",
  "name",
  "caseId",
  "case_id",
  "orderId",
  "order_id",
  "requestId",
  "request_id",
  "customerName",
  "customer_name",
  "purpose",
  "category",
  "type",
] as const;

const sensitiveInputKey = /password|secret|token|credential|authorization/i;

export function enterpriseSidebarTitle(input: {
  id: string;
  label: string;
  qualifier?: string | null;
}): string {
  const label = input.label.trim();
  const id = input.id.trim();
  const base =
    !label || normalizedIdentity(label) === normalizedIdentity(id)
      ? humanizeIdentifier(id)
      : label;
  const qualifier = input.qualifier?.trim();
  return qualifier ? `${base} · ${qualifier}` : base;
}

export function humanizeIdentifier(value: string): string {
  const words = value
    .trim()
    .replace(/([a-z\d])([A-Z])/g, "$1 $2")
    .split(/[._:@/\\-]+|\s+/)
    .filter(Boolean);
  if (words.length === 0) return value;
  return words
    .map((word) =>
      /^[a-z]/.test(word)
        ? `${word[0]?.toLocaleUpperCase()}${word.slice(1)}`
        : word,
    )
    .join(" ");
}

export function flowCaseCoreLabel(flowCase: FlowCase): string {
  const inputSummary = summarizeInput(flowCase.input);
  if (inputSummary) return inputSummary;

  const identitySegment = flowCase.idempotencyKey
    .split(/[:/]/)
    .find((segment) => /^(?:case|order|request)[_-]/i.test(segment));
  if (identitySegment) return humanizeIdentifier(identitySegment);

  return compactSidebarTime(flowCase.createdAt);
}

export function compactSidebarTime(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat(undefined, {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }).format(date);
}

export function workflowTriggerLabel(trigger: WorkflowTrigger): string {
  switch (trigger.kind) {
    case "event_subscription":
      return `${humanizeIdentifier(trigger.source)} · ${humanizeIdentifier(trigger.eventType)}`;
    case "schedule":
      return "Schedule";
    case "webhook":
      return "Webhook";
    case "manual":
      return "Manual";
  }
}

function summarizeInput(value: unknown): string | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const record = value as Record<string, unknown>;
  const values: string[] = [];

  for (const key of preferredInputKeys) {
    const label = scalarLabel(record[key]);
    if (label && !values.includes(label)) values.push(label);
    if (values.length === 2) break;
  }

  if (values.length === 0) {
    for (const [key, rawValue] of Object.entries(record)) {
      if (sensitiveInputKey.test(key)) continue;
      const label = scalarLabel(rawValue);
      if (label && !values.includes(label)) values.push(label);
      if (values.length >= 2) break;
    }
  }

  return values.length > 0 ? values.join(" · ") : null;
}

function scalarLabel(value: unknown): string | null {
  if (typeof value !== "string" && typeof value !== "number") return null;
  const label = String(value).trim();
  if (!label || label.length > 72) return null;
  return /^[\w.:@/\\-]+$/u.test(label) ? humanizeIdentifier(label) : label;
}

function normalizedIdentity(value: string): string {
  return value.toLocaleLowerCase().replace(/[^\p{L}\p{N}]+/gu, "");
}
