import Ajv, {
  type AnySchema,
  type ErrorObject,
  type ValidateFunction,
} from "ajv";
import addFormats from "ajv-formats";
import type {
  AgentActivityNotification,
  AgentEvent,
  TerminalEvent,
} from "../types";
import type { AgentActivityNotification as WireAgentActivityNotification } from "./generated/agent-activity-envelope-v1.generated";
import type { AgentEvent as WireAgentEvent } from "./generated/agent-event-envelope-v1.generated";
import type { TerminalEvent as WireTerminalEvent } from "./generated/terminal-event-envelope-v1.generated";
import agentActivityEnvelopeSchema from "./generated/agent-activity-envelope-v1.schema.json" with { type: "json" };
import agentEventEnvelopeSchema from "./generated/agent-event-envelope-v1.schema.json" with { type: "json" };
import terminalEventEnvelopeSchema from "./generated/terminal-event-envelope-v1.schema.json" with { type: "json" };

const STREAM_API_VERSION = 1;

type StreamKind = "agent_event" | "agent_activity" | "terminal_event";

type StreamEnvelope<T> = {
  apiVersion: typeof STREAM_API_VERSION;
  kind: StreamKind;
  seq: number;
  data: T;
};

const ajv = new Ajv({ allErrors: true, strict: false, useDefaults: true });
addFormats(ajv);
addIntegerFormat("int32", (value) => Number.isInteger(value));
addIntegerFormat("int64", (value) => Number.isSafeInteger(value));
addIntegerFormat("uint", isUnsignedInteger);
addIntegerFormat("uint8", (value) => isUnsignedInteger(value) && value <= 255);
addIntegerFormat(
  "uint16",
  (value) => isUnsignedInteger(value) && value <= 65_535,
);
addIntegerFormat(
  "uint32",
  (value) => isUnsignedInteger(value) && value <= 4_294_967_295,
);
addIntegerFormat(
  "uint64",
  (value) => Number.isSafeInteger(value) && value >= 0,
);

const validateAgentEventEnvelope = compileEnvelope<WireAgentEvent>(
  agentEventEnvelopeSchema,
);
const validateAgentEvent = compilePayload<WireAgentEvent>(
  agentEventEnvelopeSchema,
  "AgentEvent",
);
const validateAgentActivityEnvelope =
  compileEnvelope<WireAgentActivityNotification>(agentActivityEnvelopeSchema);
const validateAgentActivity = compilePayload<WireAgentActivityNotification>(
  agentActivityEnvelopeSchema,
  "AgentActivityNotification",
);
const validateTerminalEventEnvelope = compileEnvelope<WireTerminalEvent>(
  terminalEventEnvelopeSchema,
);
const validateTerminalEvent = compilePayload<WireTerminalEvent>(
  terminalEventEnvelopeSchema,
  "TerminalEvent",
);

export class ApiContractError extends Error {
  readonly contract: string;
  readonly validationErrors: ErrorObject[];

  constructor(
    contract: string,
    message: string,
    validationErrors: ErrorObject[] = [],
  ) {
    super(`${contract}: ${message}`);
    this.name = "ApiContractError";
    this.contract = contract;
    this.validationErrors = validationErrors;
  }
}

export function decodeAgentEvent(data: string): AgentEvent {
  const event = decodeStreamPayload(
    data,
    "agent_event",
    validateAgentEventEnvelope,
    validateAgentEvent,
  );
  const payload = event.payload;
  if (
    (payload.type === "model_context_built" ||
      payload.type === "model_request") &&
    typeof payload.request_id !== "string"
  ) {
    throw new ApiContractError(
      "agent_event",
      `${payload.type} is missing its serialized request_id`,
    );
  }
  return promoteValidatedWireValue<AgentEvent>(event);
}

export function decodeAgentActivityNotification(
  data: string,
): AgentActivityNotification {
  return promoteValidatedWireValue<AgentActivityNotification>(
    decodeStreamPayload(
      data,
      "agent_activity",
      validateAgentActivityEnvelope,
      validateAgentActivity,
    ),
  );
}

export function decodeTerminalEvent(data: string): TerminalEvent {
  return promoteValidatedWireValue<TerminalEvent>(
    decodeStreamPayload(
      data,
      "terminal_event",
      validateTerminalEventEnvelope,
      validateTerminalEvent,
    ),
  );
}

function promoteValidatedWireValue<TDomain>(value: unknown): TDomain {
  // Rust-generated types describe the legacy-compatible Serde input surface, where defaulted
  // fields are optional. Ajv materializes deterministic defaults above; this is the single
  // promotion point into Desktop's stricter internal domain types after runtime validation.
  return value as TDomain;
}

function decodeStreamPayload<T extends { seq: number }>(
  data: string,
  expectedKind: StreamKind,
  validateEnvelope: ValidateFunction<StreamEnvelope<T>>,
  validateLegacyPayload: ValidateFunction<T>,
): T {
  const parsed = parseJson(data, expectedKind);
  if (isVersionedEnvelope(parsed)) {
    if (!validateEnvelope(parsed)) {
      throw validationError(expectedKind, validateEnvelope.errors);
    }
    if (parsed.apiVersion !== STREAM_API_VERSION) {
      throw new ApiContractError(
        expectedKind,
        `unsupported apiVersion ${parsed.apiVersion}`,
      );
    }
    if (parsed.kind !== expectedKind) {
      throw new ApiContractError(
        expectedKind,
        `received envelope kind ${parsed.kind}`,
      );
    }
    if (parsed.seq !== parsed.data.seq) {
      throw new ApiContractError(
        expectedKind,
        `envelope seq ${parsed.seq} does not match payload seq ${parsed.data.seq}`,
      );
    }
    return parsed.data;
  }

  // Compatibility for a new Desktop connecting to a pre-envelope local server.
  // The legacy payload is still checked against the Rust-generated inner schema.
  if (!validateLegacyPayload(parsed)) {
    throw validationError(
      `${expectedKind} legacy payload`,
      validateLegacyPayload.errors,
    );
  }
  return parsed;
}

function parseJson(data: string, contract: string): unknown {
  try {
    return JSON.parse(data) as unknown;
  } catch (error) {
    throw new ApiContractError(
      contract,
      `invalid JSON: ${error instanceof Error ? error.message : String(error)}`,
    );
  }
}

function isVersionedEnvelope(value: unknown): value is {
  apiVersion: unknown;
  kind?: unknown;
  seq?: unknown;
  data?: unknown;
} {
  return (
    typeof value === "object" &&
    value !== null &&
    Object.prototype.hasOwnProperty.call(value, "apiVersion")
  );
}

function compileEnvelope<T>(
  schema: object,
): ValidateFunction<StreamEnvelope<T>> {
  return ajv.compile<StreamEnvelope<T>>(schema);
}

function compilePayload<T>(
  envelopeSchema: { $schema?: string; definitions?: Record<string, unknown> },
  definition: string,
): ValidateFunction<T> {
  const schema: AnySchema = {
    $schema: envelopeSchema.$schema,
    $ref: `#/definitions/${definition}`,
    definitions: envelopeSchema.definitions,
  };
  return ajv.compile<T>(schema);
}

function validationError(
  contract: string,
  errors: ErrorObject[] | null | undefined,
): ApiContractError {
  const details = errors ?? [];
  const summary = details
    .slice(0, 3)
    .map(
      (error) =>
        `${error.instancePath || "/"} ${error.message ?? "is invalid"}`,
    )
    .join("; ");
  return new ApiContractError(
    contract,
    summary || "payload is invalid",
    details,
  );
}

function addIntegerFormat(
  name: string,
  validate: (value: number) => boolean,
): void {
  ajv.addFormat(name, { type: "number", validate });
}

function isUnsignedInteger(value: number): boolean {
  return Number.isSafeInteger(value) && value >= 0;
}
