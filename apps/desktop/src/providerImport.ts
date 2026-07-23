export type ProviderImportFormat = "json" | "env" | "curl" | "unknown";

export type ImportableProviderKind = "openai_compatible" | "openai_responses";

export type ProviderImportDraft = {
  /** Suggested stable identifier. Callers should resolve collisions before saving. */
  id: string;
  /** Suggested display name derived from the input or endpoint host. */
  name: string;
  kind: ImportableProviderKind;
  baseUrl: string;
  model: string;
  apiKey?: string;
  warnings: string[];
  detectedFormat: ProviderImportFormat;
};

export type ProviderImportPresetId =
  "openai-compatible" | "openai-responses" | "ollama-local";

export type ProviderImportPreset = {
  id: ProviderImportPresetId;
  name: string;
  description: string;
  kind: ImportableProviderKind;
  baseUrl: string;
  model: string;
};

export const PROVIDER_IMPORT_PRESETS: readonly ProviderImportPreset[] = [
  {
    id: "openai-compatible",
    name: "OpenAI Compatible",
    description: "Any service exposing the OpenAI Chat Completions API.",
    kind: "openai_compatible",
    baseUrl: "https://api.openai.com/v1",
    model: "gpt-4.1-mini",
  },
  {
    id: "openai-responses",
    name: "OpenAI Responses",
    description: "OpenAI's Responses API with reasoning and response storage.",
    kind: "openai_responses",
    baseUrl: "https://api.openai.com/v1",
    model: "gpt-4.1-mini",
  },
  {
    id: "ollama-local",
    name: "Ollama Local",
    description: "A local Ollama server using its OpenAI-compatible endpoint.",
    kind: "openai_compatible",
    baseUrl: "http://localhost:11434/v1",
    model: "qwen2.5-coder:7b",
  },
] as const;

type CandidateValues = {
  id?: string;
  name?: string;
  kind?: ImportableProviderKind;
  baseUrl?: string;
  model?: string;
  apiKey?: string;
  warnings: string[];
};

type FlatEntry = {
  key: string;
  normalizedKey: string;
  path: string[];
  value: unknown;
};

const DEFAULT_PRESET = PROVIDER_IMPORT_PRESETS[0];

const KEY_ALIASES = {
  id: ["providerid", "id"],
  name: ["providername", "displayname", "name"],
  kind: ["providerkind", "providertype", "apitype", "kind", "type"],
  baseUrl: [
    "opentopiaopenaibaseurl",
    "openaibaseurl",
    "openaiapibase",
    "openaiapiurl",
    "openaiendpoint",
    "providerbaseurl",
    "baseurl",
    "apibaseurl",
    "apibase",
    "apiurl",
    "endpoint",
    "url",
  ],
  model: [
    "opentopiamodel",
    "openaimodel",
    "openaimodelname",
    "providermodel",
    "modelname",
    "model",
  ],
  apiKey: [
    "opentopiaapikey",
    "openaiapikey",
    "providerapikey",
    "bearertoken",
    "accesstoken",
    "authorization",
    "apikey",
  ],
} as const;

/**
 * Parses provider settings pasted from common configuration formats.
 *
 * This function is deterministic, has no side effects, and never writes or logs
 * the imported credential. Incomplete input produces editable defaults plus
 * warnings rather than throwing.
 */
export function parseProviderImport(input: string): ProviderImportDraft {
  const source = stripCodeFence(input.replace(/^\uFEFF/, "").trim());
  const detectedFormat = detectFormat(source);
  let candidates: CandidateValues;

  if (!source) {
    candidates = { warnings: ["Import text is empty."] };
  } else if (detectedFormat === "curl") {
    candidates = parseCurl(source);
  } else if (detectedFormat === "json") {
    candidates = parseJson(source);
  } else if (detectedFormat === "env") {
    candidates = parseEnv(source);
  } else {
    candidates = {
      warnings: [
        "Could not recognize this input as JSON, environment variables, or curl.",
      ],
    };
  }

  return normalizeDraft(candidates, detectedFormat);
}

export function getProviderImportPreset(
  presetId: ProviderImportPresetId,
): ProviderImportPreset {
  return (
    PROVIDER_IMPORT_PRESETS.find((preset) => preset.id === presetId) ??
    DEFAULT_PRESET
  );
}

export function createProviderDraftFromPreset(
  presetId: ProviderImportPresetId,
): ProviderImportDraft {
  const preset = getProviderImportPreset(presetId);
  return {
    id: preset.id,
    name: preset.name,
    kind: preset.kind,
    baseUrl: preset.baseUrl,
    model: preset.model,
    warnings: [],
    detectedFormat: "unknown",
  };
}

function detectFormat(source: string): ProviderImportFormat {
  if (!source) return "unknown";
  if (/(?:^|\s)curl(?:\.exe)?(?:\s|$)/i.test(source)) return "curl";

  if (source.startsWith("{") || source.startsWith("[")) return "json";

  if (
    source
      .split(/\r?\n/)
      .some((line) =>
        /^(?:\s*(?:(?:export|set)\s+)?[A-Za-z_][\w.-]*|\s*\$env:[A-Za-z_][\w.-]*)\s*=/i.test(
          line,
        ),
      )
  ) {
    return "env";
  }

  return "unknown";
}

function parseJson(source: string): CandidateValues {
  let parsed: unknown;
  try {
    parsed = JSON.parse(source);
  } catch {
    return { warnings: ["The JSON configuration is invalid."] };
  }

  if (!isRecord(parsed) && !Array.isArray(parsed)) {
    return { warnings: ["The JSON configuration must contain an object."] };
  }

  return candidatesFromEntries(flattenValues(parsed));
}

function parseEnv(source: string): CandidateValues {
  const entries: FlatEntry[] = [];
  const warnings: string[] = [];

  for (const rawLine of source.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line || line.startsWith("#") || line.startsWith(";")) continue;

    const match = line.match(
      /^(?:(?:(?:export|set)\s+)?([A-Za-z_][\w.-]*)|\$env:([A-Za-z_][\w.-]*))\s*=\s*(.*)$/i,
    );
    if (!match) {
      warnings.push("Some non-assignment lines were ignored.");
      continue;
    }

    const key = match[1] ?? match[2];
    entries.push({
      key,
      normalizedKey: normalizeKey(key),
      path: [key],
      value: parseAssignmentValue(match[3]),
    });
  }

  const candidates = candidatesFromEntries(entries);
  candidates.warnings.unshift(...unique(warnings));
  return candidates;
}

function parseCurl(source: string): CandidateValues {
  const tokens = tokenizeCommand(source);
  const warnings: string[] = [];
  const headers: string[] = [];
  const bodies: string[] = [];
  let requestUrl: string | undefined;

  for (let index = 1; index < tokens.length; index += 1) {
    const token = tokens[index];
    const lower = token.toLowerCase();

    if (lower === "--url") {
      requestUrl = tokens[index + 1] ?? requestUrl;
      index += 1;
    } else if (lower.startsWith("--url=")) {
      requestUrl = token.slice(token.indexOf("=") + 1);
    } else if (lower === "-h" || lower === "--header") {
      if (tokens[index + 1]) headers.push(tokens[index + 1]);
      index += 1;
    } else if (lower.startsWith("--header=")) {
      headers.push(token.slice(token.indexOf("=") + 1));
    } else if (
      lower === "-d" ||
      lower === "--data" ||
      lower === "--data-raw" ||
      lower === "--data-binary" ||
      lower === "--json"
    ) {
      if (tokens[index + 1]) bodies.push(tokens[index + 1]);
      index += 1;
    } else if (
      lower.startsWith("--data=") ||
      lower.startsWith("--data-raw=") ||
      lower.startsWith("--data-binary=") ||
      lower.startsWith("--json=")
    ) {
      bodies.push(token.slice(token.indexOf("=") + 1));
    } else if (
      !token.startsWith("-") &&
      /^https?:\/\//i.test(token) &&
      !requestUrl
    ) {
      requestUrl = token;
    }
  }

  let apiKey: string | undefined;
  for (const header of headers) {
    const bearer = header.match(/^\s*authorization\s*:\s*bearer\s+(.+?)\s*$/i);
    const apiKeyHeader = header.match(
      /^\s*(?:x-api-key|api-key)\s*:\s*(.+?)\s*$/i,
    );
    const value = bearer?.[1] ?? apiKeyHeader?.[1];
    if (value) {
      const resolved = cleanSecret(value);
      if (isVariableReference(resolved)) {
        warnings.push(
          "The curl credential is a variable reference, so no API key was imported.",
        );
      } else {
        apiKey = resolved;
      }
      break;
    }
  }

  let bodyCandidates: CandidateValues = { warnings: [] };
  for (const body of bodies) {
    try {
      const parsed: unknown = JSON.parse(body);
      if (isRecord(parsed) || Array.isArray(parsed)) {
        bodyCandidates = candidatesFromEntries(flattenValues(parsed));
        if (bodyCandidates.model) break;
      }
    } catch {
      // curl bodies often contain non-JSON prompt data; only flag this when no
      // usable JSON body is found at all.
    }
  }

  if (bodies.length > 0 && !bodyCandidates.model) {
    warnings.push("No model was found in the curl JSON body.");
  }

  const kind = inferKind(bodyCandidates.kind, requestUrl);
  return {
    ...bodyCandidates,
    kind,
    baseUrl: requestUrl,
    apiKey: apiKey ?? bodyCandidates.apiKey,
    warnings: [...warnings, ...bodyCandidates.warnings],
  };
}

function candidatesFromEntries(entries: FlatEntry[]): CandidateValues {
  const kindValue = findEntry(entries, KEY_ALIASES.kind);
  const responseFlag = entries.find(
    (entry) =>
      ["useresponses", "responsesapi", "storeresponses"].includes(
        entry.normalizedKey,
      ) && entry.value === true,
  );

  return {
    id: toOptionalString(findEntry(entries, KEY_ALIASES.id), false),
    name: toOptionalString(findEntry(entries, KEY_ALIASES.name), false),
    kind: inferKind(
      toOptionalString(kindValue, false),
      undefined,
      Boolean(responseFlag),
    ),
    baseUrl: toOptionalString(findEntry(entries, KEY_ALIASES.baseUrl), false),
    model: toOptionalString(findEntry(entries, KEY_ALIASES.model), false),
    apiKey: toOptionalString(findEntry(entries, KEY_ALIASES.apiKey), true),
    warnings: [],
  };
}

function normalizeDraft(
  candidates: CandidateValues,
  detectedFormat: ProviderImportFormat,
): ProviderImportDraft {
  const warnings = [...candidates.warnings];
  const endpointKind = inferKind(candidates.kind, candidates.baseUrl);
  const kind = endpointKind ?? DEFAULT_PRESET.kind;
  const baseUrl = normalizeBaseUrl(candidates.baseUrl, warnings);
  const model = candidates.model?.trim() || defaultModelFor(baseUrl);

  if (!candidates.baseUrl?.trim()) {
    warnings.push(
      "No base URL was found; the OpenAI-compatible default was used.",
    );
  }
  if (!candidates.model?.trim()) {
    warnings.push(
      "No model was found; choose the correct model before saving.",
    );
  }

  const suggestedName =
    candidates.name?.trim() || suggestNameFromUrl(baseUrl, kind);
  const idSource = candidates.id?.trim() || suggestedName;
  const id = slugify(idSource) || "custom-provider";
  const apiKey = cleanSecret(candidates.apiKey ?? "");

  if (apiKey && isVariableReference(apiKey)) {
    warnings.push(
      "The credential is a variable reference, so no API key was imported.",
    );
  }

  return {
    id,
    name: suggestedName,
    kind,
    baseUrl,
    model,
    ...(apiKey && !isVariableReference(apiKey) ? { apiKey } : {}),
    warnings: unique(warnings),
    detectedFormat,
  };
}

function normalizeBaseUrl(
  rawValue: string | undefined,
  warnings: string[],
): string {
  if (!rawValue?.trim()) return DEFAULT_PRESET.baseUrl;

  let value = trimWrappingQuotes(rawValue.trim());
  if (!/^https?:\/\//i.test(value)) {
    const local =
      /^(?:localhost|127\.0\.0\.1|0\.0\.0\.0|\[::1\])(?::|\/|$)/i.test(value);
    value = `${local ? "http" : "https"}://${value}`;
    warnings.push("The base URL had no protocol, so one was added.");
  }

  try {
    const url = new URL(value);
    if (url.protocol !== "http:" && url.protocol !== "https:") {
      warnings.push(
        "The base URL must use HTTP or HTTPS; the default was used.",
      );
      return DEFAULT_PRESET.baseUrl;
    }

    const nativeOllamaPath = /^\/api\/(?:chat|generate)\/?$/i.test(
      url.pathname,
    );
    if (nativeOllamaPath) {
      url.pathname = "/v1";
      warnings.push(
        "The native Ollama endpoint was converted to its OpenAI-compatible /v1 endpoint.",
      );
    } else {
      url.pathname = url.pathname.replace(
        /\/(?:chat\/completions|responses|models)\/?$/i,
        "",
      );
    }

    url.search = "";
    url.hash = "";
    return url.toString().replace(/\/$/, "");
  } catch {
    warnings.push("The base URL is invalid; the default was used.");
    return DEFAULT_PRESET.baseUrl;
  }
}

function inferKind(
  rawKind: ImportableProviderKind | string | undefined,
  url?: string,
  responseFlag = false,
): ImportableProviderKind | undefined {
  const value = rawKind?.toLowerCase().replace(/[^a-z]/g, "") ?? "";
  if (
    responseFlag ||
    value.includes("response") ||
    /\/responses(?:[/?#]|$)/i.test(url ?? "")
  ) {
    return "openai_responses";
  }
  if (
    value.includes("openai") ||
    value.includes("compatible") ||
    value.includes("chatcompletion")
  ) {
    return "openai_compatible";
  }
  return undefined;
}

function flattenValues(value: unknown, path: string[] = []): FlatEntry[] {
  if (Array.isArray(value)) {
    return value.flatMap((item, index) =>
      flattenValues(item, [...path, String(index)]),
    );
  }
  if (!isRecord(value)) return [];

  const entries: FlatEntry[] = [];
  for (const [key, child] of Object.entries(value)) {
    const childPath = [...path, key];
    if (isRecord(child) || Array.isArray(child)) {
      entries.push(...flattenValues(child, childPath));
    } else {
      entries.push({
        key,
        normalizedKey: normalizeKey(key),
        path: childPath,
        value: child,
      });
    }
  }
  return entries;
}

function findEntry(entries: FlatEntry[], aliases: readonly string[]): unknown {
  for (const alias of aliases) {
    const matches = entries.filter((entry) => entry.normalizedKey === alias);
    if (matches.length === 0) continue;
    // Prefer a root value, then the least deeply nested match. This avoids a
    // request payload overriding top-level provider configuration.
    matches.sort((left, right) => left.path.length - right.path.length);
    return matches[0].value;
  }
  return undefined;
}

function tokenizeCommand(source: string): string[] {
  const normalized = source
    .replace(/\\\r?\n/g, " ")
    .replace(/`\r?\n/g, " ")
    .replace(/\^\r?\n/g, " ");
  const tokens: string[] = [];
  let token = "";
  let quote: "'" | '"' | null = null;

  for (let index = 0; index < normalized.length; index += 1) {
    const character = normalized[index];
    if (quote) {
      if (character === quote) {
        quote = null;
      } else if (
        character === "\\" &&
        quote === '"' &&
        index + 1 < normalized.length
      ) {
        token += normalized[index + 1];
        index += 1;
      } else {
        token += character;
      }
    } else if (character === "'" || character === '"') {
      quote = character;
    } else if (/\s/.test(character)) {
      if (token) {
        tokens.push(token);
        token = "";
      }
    } else {
      token += character;
    }
  }

  if (token) tokens.push(token);
  return tokens;
}

function parseAssignmentValue(rawValue: string): string {
  const trimmed = stripInlineComment(rawValue).trim();
  if (!trimmed) return "";
  if (
    (trimmed.startsWith('"') && trimmed.endsWith('"')) ||
    (trimmed.startsWith("'") && trimmed.endsWith("'"))
  ) {
    return trimWrappingQuotes(trimmed);
  }
  return trimmed;
}

function stripInlineComment(value: string): string {
  let quote: "'" | '"' | null = null;
  for (let index = 0; index < value.length; index += 1) {
    const character = value[index];
    if (quote) {
      if (character === quote && value[index - 1] !== "\\") quote = null;
    } else if (character === "'" || character === '"') {
      quote = character;
    } else if (
      character === "#" &&
      (index === 0 || /\s/.test(value[index - 1]))
    ) {
      return value.slice(0, index);
    }
  }
  return value;
}

function stripCodeFence(value: string): string {
  const match = value.match(
    /^```(?:json|jsonc|dotenv|env|bash|sh|shell|powershell)?\s*\r?\n([\s\S]*?)\r?\n```$/i,
  );
  return match?.[1].trim() ?? value;
}

function trimWrappingQuotes(value: string): string {
  if (
    value.length >= 2 &&
    ((value.startsWith('"') && value.endsWith('"')) ||
      (value.startsWith("'") && value.endsWith("'")))
  ) {
    return value.slice(1, -1);
  }
  return value;
}

function cleanSecret(value: string): string {
  return trimWrappingQuotes(value.trim())
    .replace(/^bearer\s+/i, "")
    .trim();
}

function isVariableReference(value: string): boolean {
  return (
    /^\$\{?[A-Za-z_][A-Za-z0-9_]*\}?$/.test(value) ||
    /^%[A-Za-z_][A-Za-z0-9_]*%$/.test(value) ||
    /^\$env:[A-Za-z_][A-Za-z0-9_]*$/i.test(value)
  );
}

function toOptionalString(
  value: unknown,
  stripBearerPrefix: boolean,
): string | undefined {
  if (typeof value !== "string" && typeof value !== "number") return undefined;
  const stringValue = String(value).trim();
  if (!stringValue) return undefined;
  return stripBearerPrefix ? cleanSecret(stringValue) : stringValue;
}

function suggestNameFromUrl(
  baseUrl: string,
  kind: ImportableProviderKind,
): string {
  try {
    const url = new URL(baseUrl);
    const host = url.hostname.toLowerCase();
    if (host === "api.openai.com") {
      return kind === "openai_responses" ? "OpenAI Responses" : "OpenAI";
    }
    if (
      host === "localhost" ||
      host === "127.0.0.1" ||
      host === "0.0.0.0" ||
      host === "[::1]"
    ) {
      return url.port === "11434" ? "Ollama Local" : "Local Provider";
    }

    const label = host
      .replace(/^www\./, "")
      .replace(/^api\./, "")
      .split(".")[0];
    return titleCase(label) || "Custom Provider";
  } catch {
    return "Custom Provider";
  }
}

function defaultModelFor(baseUrl: string): string {
  try {
    const url = new URL(baseUrl);
    if (
      url.port === "11434" &&
      ["localhost", "127.0.0.1", "0.0.0.0", "[::1]"].includes(
        url.hostname.toLowerCase(),
      )
    ) {
      return getProviderImportPreset("ollama-local").model;
    }
  } catch {
    // normalizeBaseUrl already replaces invalid URLs with a valid default.
  }
  return DEFAULT_PRESET.model;
}

function slugify(value: string): string {
  return value
    .normalize("NFKD")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 64);
}

function titleCase(value: string): string {
  return value
    .split(/[-_\s]+/)
    .filter(Boolean)
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(" ");
}

function normalizeKey(value: string): string {
  return value.toLowerCase().replace(/[^a-z0-9]/g, "");
}

function unique(values: string[]): string[] {
  return [...new Set(values)];
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
