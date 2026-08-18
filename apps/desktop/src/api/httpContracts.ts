import Ajv, { type ErrorObject, type ValidateFunction } from "ajv";
import addFormats from "ajv-formats";
import type { DesktopHttpResponsesV1 } from "./generated/desktop-http-v1.generated";
import desktopHttpSchema from "./generated/desktop-http-v1.schema.json" with { type: "json" };
import { ApiContractError } from "./sseContracts.ts";

export type HttpContractKey = Extract<keyof DesktopHttpResponsesV1, string>;

type HttpSchema = {
  $schema?: string;
  definitions?: Record<string, unknown>;
  properties?: Record<string, unknown>;
};

const schema = desktopHttpSchema as HttpSchema;
const ajv = new Ajv({ allErrors: true, strict: false, useDefaults: true });
addFormats(ajv);
addIntegerFormats();

const validators = new Map<HttpContractKey, ValidateFunction>();

export function decodeHttpResponse<T>(
  contract: HttpContractKey,
  value: unknown,
): T {
  const validate = validatorFor(contract);
  if (!validate(value)) {
    throw validationError(contract, validate.errors);
  }

  // Rust-generated schemas describe the Serde wire surface. Ajv has applied
  // defaults, so this is the sole promotion point into Desktop domain types.
  return value as T;
}

export function parseHttpResponseJson(
  contract: HttpContractKey,
  text: string,
): unknown {
  try {
    return JSON.parse(text) as unknown;
  } catch (error) {
    throw new ApiContractError(
      contract,
      `invalid JSON: ${error instanceof Error ? error.message : String(error)}`,
    );
  }
}

function validatorFor(contract: HttpContractKey): ValidateFunction {
  const existing = validators.get(contract);
  if (existing) return existing;

  if (!schema.properties?.[contract]) {
    throw new ApiContractError(contract, "contract is not registered");
  }

  const validate = ajv.compile({
    $schema: schema.$schema,
    $ref: `#/properties/${escapeJsonPointer(contract)}`,
    definitions: schema.definitions,
    properties: schema.properties,
  });
  validators.set(contract, validate);
  return validate;
}

function validationError(
  contract: HttpContractKey,
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

function addIntegerFormats(): void {
  const add = (name: string, validate: (value: number) => boolean) =>
    ajv.addFormat(name, { type: "number", validate });
  const unsigned = (value: number) => Number.isSafeInteger(value) && value >= 0;
  add("int32", Number.isInteger);
  add("int64", Number.isSafeInteger);
  add("uint", unsigned);
  add("uint8", (value) => unsigned(value) && value <= 255);
  add("uint16", (value) => unsigned(value) && value <= 65_535);
  add("uint32", (value) => unsigned(value) && value <= 4_294_967_295);
  add("uint64", unsigned);
}

function escapeJsonPointer(value: string): string {
  return value.replaceAll("~", "~0").replaceAll("/", "~1");
}
