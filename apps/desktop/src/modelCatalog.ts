import type { ProviderKind, ReasoningEffort } from "./types";
import {
  REASONING_EFFORT_DETAILS,
  resolveModelReasoningCapability,
  type ModelReasoningCapability,
} from "./providerSettings";

export { REASONING_EFFORT_DETAILS };
export type { ModelReasoningCapability };

/**
 * A model family is the unit users enable per connection. Relay endpoints
 * ("中转站") expose many vendors behind one API key, so the family — not the
 * individual model id — is the thing worth toggling.
 */
export type ModelFamilyId =
  | "gpt"
  | "claude"
  | "gemini"
  | "kimi"
  | "deepseek"
  | "qwen"
  | "glm"
  | "minimax"
  | "grok"
  | "llama"
  | "mistral"
  | "other";

export type ModelFamily = {
  id: ModelFamilyId;
  label: string;
  vendor: string;
};

export const MODEL_FAMILIES: readonly ModelFamily[] = [
  { id: "gpt", label: "GPT", vendor: "OpenAI" },
  { id: "claude", label: "Claude", vendor: "Anthropic" },
  { id: "gemini", label: "Gemini", vendor: "Google" },
  { id: "kimi", label: "Kimi", vendor: "Moonshot AI" },
  { id: "deepseek", label: "DeepSeek", vendor: "DeepSeek" },
  { id: "qwen", label: "Qwen", vendor: "Alibaba" },
  { id: "glm", label: "GLM", vendor: "Zhipu AI" },
  { id: "minimax", label: "MiniMax", vendor: "MiniMax" },
  { id: "grok", label: "Grok", vendor: "xAI" },
  { id: "llama", label: "Llama", vendor: "Meta" },
  { id: "mistral", label: "Mistral", vendor: "Mistral AI" },
  { id: "other", label: "其他", vendor: "自定义" },
] as const;

const FAMILY_BY_ID = new Map<ModelFamilyId, ModelFamily>(
  MODEL_FAMILIES.map((family) => [family.id, family]),
);

export function getModelFamily(id: ModelFamilyId): ModelFamily {
  return FAMILY_BY_ID.get(id) ?? FAMILY_BY_ID.get("other")!;
}

/**
 * Patterns are ordered: the first match wins. They are deliberately loose
 * because relay endpoints rename models freely (`moonshot/kimi-k2`,
 * `anthropic.claude-sonnet-4`, `qwen-max-latest`). Anything unmatched still
 * resolves to the `other` family and stays usable.
 */
const FAMILY_PATTERNS: ReadonlyArray<{
  pattern: RegExp;
  familyId: ModelFamilyId;
}> = [
  { pattern: /(?:^|[/\-.])(?:gpt|chatgpt)[-\d]/, familyId: "gpt" },
  { pattern: /(?:^|[/\-.])o[134](?:$|[-\d])/, familyId: "gpt" },
  { pattern: /(?:^|[/\-.])claude(?:$|[-\d.])/, familyId: "claude" },
  { pattern: /(?:^|[/\-.])gemini(?:$|[-\d.])/, familyId: "gemini" },
  { pattern: /(?:^|[/\-.])(?:kimi|moonshot)(?:$|[-\d.v])/, familyId: "kimi" },
  { pattern: /(?:^|[/\-.])deepseek(?:$|[-\d.])/, familyId: "deepseek" },
  { pattern: /(?:^|[/\-.])(?:qwen|qwq|qvq)(?:$|[-\d.])/, familyId: "qwen" },
  { pattern: /(?:^|[/\-.])(?:glm|chatglm)(?:$|[-\d.])/, familyId: "glm" },
  { pattern: /(?:^|[/\-.])(?:minimax|abab)(?:$|[-\d.])/, familyId: "minimax" },
  { pattern: /(?:^|[/\-.])grok(?:$|[-\d.])/, familyId: "grok" },
  { pattern: /(?:^|[/\-.])(?:llama|codellama)(?:$|[-\d.])/, familyId: "llama" },
  {
    pattern: /(?:^|[/\-.])(?:mistral|mixtral|codestral|magistral)(?:$|[-\d.])/,
    familyId: "mistral",
  },
];

export function classifyModelFamily(modelId: string): ModelFamilyId {
  const normalized = modelId.trim().toLocaleLowerCase();
  return (
    FAMILY_PATTERNS.find(({ pattern }) => pattern.test(normalized))?.familyId ??
    "other"
  );
}

const PREVIEW_MARKERS =
  /(?:^|[/\-.])(?:preview|beta|alpha|exp|experimental|nightly|snapshot|test|draft|rc\d*)(?:$|[-\d.])/;

export function isPreviewModel(modelId: string): boolean {
  return PREVIEW_MARKERS.test(modelId.trim().toLocaleLowerCase());
}

/** Vendor prefixes relays prepend; stripped for display only, never when calling. */
const VENDOR_PREFIX =
  /^(?:openai|anthropic|google|moonshot(?:ai)?|deepseek|alibaba|qwen|zhipu|minimax|xai|meta(?:-llama)?|mistralai|azure|bedrock|vertex|openrouter|accounts)[/.]/;

const DISPLAY_TOKEN_OVERRIDES: Readonly<Record<string, string>> = {
  gpt: "GPT",
  glm: "GLM",
  qwq: "QwQ",
  qvq: "QvQ",
  ai: "AI",
  hd: "HD",
  vl: "VL",
  moe: "MoE",
  oss: "OSS",
  latest: "Latest",
  codex: "Codex",
  chat: "Chat",
  instruct: "Instruct",
  thinking: "Thinking",
  reasoner: "Reasoner",
  turbo: "Turbo",
  flash: "Flash",
  pro: "Pro",
  max: "Max",
  mini: "mini",
  nano: "nano",
  opus: "Opus",
  sonnet: "Sonnet",
  haiku: "Haiku",
};

/**
 * Best-effort prettifier for arbitrary relay model ids. It never has to be
 * perfect — the raw id stays visible as a secondary line in the picker.
 */
export function formatModelDisplayName(modelId: string): string {
  const trimmed = modelId.trim();
  if (!trimmed) return trimmed;
  const withoutVendor = trimmed.replace(VENDOR_PREFIX, "");
  const dateStripped = withoutVendor.replace(/-(\d{8}|\d{4}-\d{2}-\d{2})$/, "");
  return dateStripped
    .split(/[-_\s]+/)
    .filter(Boolean)
    .map((token) => {
      const lower = token.toLocaleLowerCase();
      if (DISPLAY_TOKEN_OVERRIDES[lower]) return DISPLAY_TOKEN_OVERRIDES[lower];
      // Version-ish tokens keep their original shape: k2.5, 4.1, v1, 5.6
      if (/\d/.test(token)) return token.toLocaleUpperCase();
      return token.charAt(0).toLocaleUpperCase() + token.slice(1);
    })
    .join(" ");
}

/**
 * Sortable recency key. Later releases must sort first so a family's default
 * lands on its newest stable model without a curated per-model list.
 */
function recencyKey(modelId: string): { version: number[]; date: number } {
  const normalized = modelId.trim().toLocaleLowerCase();

  const dateMatch = normalized.match(/(20\d{2})-?(\d{2})-?(\d{2})/);
  const date = dateMatch
    ? Number(`${dateMatch[1]}${dateMatch[2]}${dateMatch[3]}`)
    : 0;

  const withoutDate = dateMatch
    ? normalized.replace(dateMatch[0], "")
    : normalized;
  // First numeric run that looks like a version: 5.6, 2.5, 4-5, 3
  const versionMatch = withoutDate.match(/(\d+(?:[.\-]\d+)*)/);
  const version = versionMatch
    ? versionMatch[1].split(/[.\-]/).map((part) => Number(part) || 0)
    : [];

  return { version, date };
}

export function compareModelRecency(a: string, b: string): number {
  const left = recencyKey(a);
  const right = recencyKey(b);
  const length = Math.max(left.version.length, right.version.length);
  for (let index = 0; index < length; index += 1) {
    const diff = (right.version[index] ?? 0) - (left.version[index] ?? 0);
    if (diff !== 0) return diff;
  }
  if (right.date !== left.date) return right.date - left.date;
  return a.localeCompare(b);
}

export type CatalogModel = {
  /** Exact id to send to the API. Never prettified. */
  id: string;
  familyId: ModelFamilyId;
  displayName: string;
  preview: boolean;
  /** Newest stable model within its family for this connection. */
  latest: boolean;
};

export type ConnectionModelGroup = {
  family: ModelFamily;
  models: CatalogModel[];
};

/**
 * Builds the grouped, ordered model list for one connection.
 *
 * `enabledFamilies` is a per-connection allow-list. An empty array means the
 * user has not narrowed anything down yet, so everything stays visible.
 */
export function buildConnectionModelGroups(
  modelIds: readonly string[],
  enabledFamilies: readonly string[],
): ConnectionModelGroup[] {
  const unique = Array.from(
    new Map(
      modelIds
        .map((id) => id.trim())
        .filter(Boolean)
        .map((id) => [id, id]),
    ).values(),
  );

  const byFamily = new Map<ModelFamilyId, string[]>();
  for (const id of unique) {
    const familyId = classifyModelFamily(id);
    const bucket = byFamily.get(familyId);
    if (bucket) bucket.push(id);
    else byFamily.set(familyId, [id]);
  }

  const allowAll = enabledFamilies.length === 0;
  const allowed = new Set(enabledFamilies);

  return MODEL_FAMILIES.filter(
    (family) =>
      byFamily.has(family.id) && (allowAll || allowed.has(family.id)),
  ).map((family) => {
    const ids = byFamily.get(family.id)!.slice().sort(compareModelRecency);
    const latestStableId = ids.find((id) => !isPreviewModel(id)) ?? ids[0];
    return {
      family,
      models: ids.map((id) => ({
        id,
        familyId: family.id,
        displayName: formatModelDisplayName(id),
        preview: isPreviewModel(id),
        latest: id === latestStableId,
      })),
    };
  });
}

/**
 * The families a connection actually offers, used to render the settings
 * toggles. Derived from synced ids so users only see what their relay serves.
 */
export function availableFamiliesForModels(
  modelIds: readonly string[],
): ModelFamily[] {
  const seen = new Set<ModelFamilyId>(
    modelIds.map((id) => classifyModelFamily(id)),
  );
  return MODEL_FAMILIES.filter((family) => seen.has(family.id));
}

/**
 * Default model for a new thread: the newest stable model of the first enabled
 * family, falling back to the connection's configured default model.
 */
export function resolveDefaultModelId(
  modelIds: readonly string[],
  enabledFamilies: readonly string[],
  fallbackModelId: string,
): string {
  const groups = buildConnectionModelGroups(modelIds, enabledFamilies);
  for (const group of groups) {
    const latest = group.models.find((model) => model.latest);
    if (latest) return latest.id;
  }
  return fallbackModelId;
}

/**
 * Reasoning efforts valid for a model. Delegates to the existing
 * provider-kind-aware capability rules so OpenAI stays authoritative, and
 * layers family defaults on top for everything else.
 */
export function resolveReasoningOptions(
  providerKind: ProviderKind,
  modelId: string,
): ModelReasoningCapability {
  const capability = resolveModelReasoningCapability(providerKind, modelId);
  if (capability.official) return capability;

  const familyId = classifyModelFamily(modelId);
  const familyCapability = FAMILY_REASONING[familyId];
  if (familyCapability) {
    const normalized = modelId.trim().toLocaleLowerCase();
    // Non-reasoning members of an otherwise-reasoning family.
    if (familyCapability.nonReasoningPattern?.test(normalized)) {
      return {
        status: "unsupported",
        supportedEfforts: [],
        defaultEffort: null,
        sourceUrl: null,
        official: false,
      };
    }
    return {
      status: "supported",
      supportedEfforts: familyCapability.efforts,
      defaultEffort: familyCapability.defaultEffort,
      sourceUrl: null,
      official: false,
    };
  }

  return capability;
}

const FAMILY_REASONING: Partial<
  Record<
    ModelFamilyId,
    {
      efforts: readonly ReasoningEffort[];
      defaultEffort: ReasoningEffort | null;
      nonReasoningPattern?: RegExp;
    }
  >
> = {
  kimi: {
    efforts: ["none", "low", "medium", "high"],
    defaultEffort: "medium",
    nonReasoningPattern: /moonshot-v1/,
  },
  deepseek: {
    efforts: ["none", "low", "medium", "high"],
    defaultEffort: "medium",
    nonReasoningPattern: /deepseek-(?:chat|coder)(?:$|-)/,
  },
  qwen: {
    efforts: ["none", "low", "medium", "high"],
    defaultEffort: "medium",
  },
  glm: {
    efforts: ["none", "low", "medium", "high"],
    defaultEffort: "medium",
  },
  claude: {
    efforts: ["none", "low", "medium", "high", "max"],
    defaultEffort: "medium",
    nonReasoningPattern: /claude-(?:3(?:\.5)?|instant)/,
  },
  gemini: {
    efforts: ["none", "low", "medium", "high"],
    defaultEffort: "medium",
  },
  grok: {
    efforts: ["none", "low", "high"],
    defaultEffort: "low",
  },
};

/**
 * Keeps a stored effort valid when the user switches models. Returns the
 * model's default when the previous effort is not offered by the new model.
 */
export function reconcileReasoningEffort(
  providerKind: ProviderKind,
  modelId: string,
  currentEffort: ReasoningEffort | null | undefined,
): ReasoningEffort | null {
  const capability = resolveReasoningOptions(providerKind, modelId);
  if (capability.supportedEfforts.length === 0) return null;
  if (currentEffort && capability.supportedEfforts.includes(currentEffort)) {
    return currentEffort;
  }
  return capability.defaultEffort;
}
