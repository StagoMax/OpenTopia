import type {
  PluginActivationRecord,
  PluginActivationScope,
  PluginControlScope,
  PluginPermissionGrantRecord,
} from "./types";

export type PluginSettingFieldKind =
  "boolean" | "enum" | "integer" | "number" | "secret" | "string" | "json";

export type PluginSettingField = {
  key: string;
  label: string;
  description?: string;
  kind: PluginSettingFieldKind;
  required: boolean;
  defaultValue?: unknown;
  enumValues: string[];
  minimum?: number;
  maximum?: number;
};

export function pluginSettingFields(
  schema: unknown,
  secretKeys: readonly string[],
): PluginSettingField[] {
  if (!isRecord(schema) || !isRecord(schema.properties)) return [];
  const required = new Set(
    Array.isArray(schema.required)
      ? schema.required.filter(
          (value): value is string => typeof value === "string",
        )
      : [],
  );
  const secrets = new Set(secretKeys);

  return Object.entries(schema.properties).map(([key, rawProperty]) => {
    const property = isRecord(rawProperty) ? rawProperty : {};
    const enumValues = Array.isArray(property.enum)
      ? property.enum.filter(
          (value): value is string => typeof value === "string",
        )
      : [];
    const type = schemaType(property.type);
    const kind: PluginSettingFieldKind = secrets.has(key)
      ? "secret"
      : enumValues.length
        ? "enum"
        : type === "boolean" ||
            type === "integer" ||
            type === "number" ||
            type === "string"
          ? type
          : "json";
    return {
      key,
      label: readString(property.title) ?? humanizeSettingKey(key),
      description: readString(property.description),
      kind,
      required: required.has(key),
      defaultValue: property.default,
      enumValues,
      minimum: readNumber(property.minimum),
      maximum: readNumber(property.maximum),
    };
  });
}

export function scopeMatches(
  left: PluginControlScope | PluginActivationScope,
  right: PluginControlScope | PluginActivationScope,
): boolean {
  return (
    left.scopeType === right.scopeType &&
    normalizeScopeId(left.scopeId) === normalizeScopeId(right.scopeId)
  );
}

export function activationForScope(
  activations: readonly PluginActivationRecord[],
  scope: PluginActivationScope,
): PluginActivationRecord | undefined {
  return activations.find((activation) =>
    scopeMatches(activation.scope, scope),
  );
}

export function permissionGrantForScope(
  grants: readonly PluginPermissionGrantRecord[],
  scope: PluginControlScope,
  permission: string,
): PluginPermissionGrantRecord | undefined {
  return grants.find(
    (grant) =>
      grant.permission === permission && scopeMatches(grant.scope, scope),
  );
}

export function parseJsonObject(value: string): Record<string, unknown> {
  const parsed: unknown = JSON.parse(value);
  if (!isRecord(parsed)) throw new Error("Value must be a JSON object.");
  return parsed;
}

function normalizeScopeId(value: string | undefined): string {
  return (value ?? "")
    .trim()
    .replaceAll("\\", "/")
    .replace(/\/+$/, "")
    .toLocaleLowerCase();
}

function schemaType(value: unknown): string | undefined {
  if (typeof value === "string") return value;
  if (Array.isArray(value)) {
    return value.find(
      (item): item is string => item !== "null" && typeof item === "string",
    );
  }
  return undefined;
}

function humanizeSettingKey(key: string): string {
  return key
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .replace(/[._-]+/g, " ")
    .replace(/^./, (character) => character.toLocaleUpperCase());
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function readString(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value : undefined;
}

function readNumber(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value)
    ? value
    : undefined;
}
