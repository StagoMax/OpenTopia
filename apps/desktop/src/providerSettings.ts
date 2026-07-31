import type { ProviderKind, ProviderSettings, ReasoningEffort } from "./types";

export const MAX_PROVIDER_NAME_LENGTH = 80;

export const OPENAI_MODEL_GUIDE_URL =
  "https://developers.openai.com/api/docs/guides/latest-model";
export const OPENAI_REASONING_GUIDE_URL =
  "https://developers.openai.com/api/docs/guides/reasoning#reasoning-effort";
export const ANTHROPIC_EFFORT_GUIDE_URL =
  "https://platform.claude.com/docs/en/build-with-claude/effort";
export const KIMI_K3_MODEL_GUIDE_URL =
  "https://www.kimi.com/code/docs/en/kimi-code/models.html";
export const GLM_THINKING_GUIDE_URL =
  "https://docs.bigmodel.cn/cn/guide/capabilities/thinking";
export const OPENAI_MODEL_CATALOG_VERIFIED_AT = "2026-07-30";

export type OfficialModelPreset = {
  model: string;
  label: string;
  description: string;
  group: "recommended" | "compatibility";
  sourceUrl: string;
};

export type ModelReasoningCapability = {
  status: "supported" | "unsupported" | "unknown";
  supportedEfforts: readonly ReasoningEffort[];
  defaultEffort: ReasoningEffort | null;
  sourceUrl: string | null;
  official: boolean;
};

export const REASONING_EFFORT_DETAILS: Readonly<
  Record<ReasoningEffort, { label: string }>
> = {
  none: { label: "None" },
  minimal: { label: "Minimal" },
  low: { label: "Low" },
  medium: { label: "Medium" },
  high: { label: "High" },
  xhigh: { label: "XHigh" },
  max: { label: "Max" },
};

const ALL_REASONING_EFFORTS = [
  "none",
  "minimal",
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
] as const satisfies readonly ReasoningEffort[];

const GPT_5_6_EFFORTS = [
  "none",
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
] as const satisfies readonly ReasoningEffort[];
const GPT_5_4_EFFORTS = [
  "none",
  "low",
  "medium",
  "high",
  "xhigh",
] as const satisfies readonly ReasoningEffort[];
const GPT_5_3_CODEX_EFFORTS = [
  "low",
  "medium",
  "high",
  "xhigh",
] as const satisfies readonly ReasoningEffort[];
const GPT_5_1_EFFORTS = [
  "none",
  "low",
  "medium",
  "high",
] as const satisfies readonly ReasoningEffort[];
const GPT_5_EFFORTS = [
  "minimal",
  "low",
  "medium",
  "high",
] as const satisfies readonly ReasoningEffort[];
const KIMI_K3_EFFORTS = [
  "none",
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
] as const satisfies readonly ReasoningEffort[];
const CLAUDE_46_EFFORTS = [
  "low",
  "medium",
  "high",
  "max",
] as const satisfies readonly ReasoningEffort[];
const CLAUDE_XHIGH_EFFORTS = [
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
] as const satisfies readonly ReasoningEffort[];

export const OFFICIAL_OPENAI_MODEL_PRESETS: readonly OfficialModelPreset[] = [
  {
    model: "gpt-5.6-sol",
    label: "GPT-5.6 Sol",
    description: "旗舰能力，适合复杂编码、研究和高价值工作。",
    group: "recommended",
    sourceUrl: OPENAI_MODEL_GUIDE_URL,
  },
  {
    model: "gpt-5.6-terra",
    label: "GPT-5.6 Terra",
    description: "智能与成本均衡，适合大多数生产工作负载。",
    group: "recommended",
    sourceUrl: OPENAI_MODEL_GUIDE_URL,
  },
  {
    model: "gpt-5.6-luna",
    label: "GPT-5.6 Luna",
    description: "面向高吞吐和成本敏感任务的高效选择。",
    group: "recommended",
    sourceUrl: OPENAI_MODEL_GUIDE_URL,
  },
  {
    model: "gpt-5.6",
    label: "GPT-5.6（Sol 别名）",
    description: "跟随当前 GPT-5.6 Sol，便于持续使用旗舰别名。",
    group: "recommended",
    sourceUrl: OPENAI_MODEL_GUIDE_URL,
  },
  {
    model: "gpt-5.4-mini",
    label: "GPT-5.4 mini",
    description: "兼容旧项目的高吞吐 GPT-5.4 系列模型。",
    group: "compatibility",
    sourceUrl: "https://developers.openai.com/api/docs/models/gpt-5.4-mini",
  },
  {
    model: "gpt-4.1-mini",
    label: "GPT-4.1 mini",
    description: "兼容现有非推理配置，不提供推理强度参数。",
    group: "compatibility",
    sourceUrl: "https://developers.openai.com/api/docs/models/gpt-4.1-mini",
  },
] as const;

const modelCapabilityRules: ReadonlyArray<{
  pattern: RegExp;
  capability: ModelReasoningCapability;
}> = [
  {
    // Kimi maps `medium` to high and `xhigh` to max. `none` is its explicit
    // thinking-off control; `minimal` is not an accepted K3 effort value.
    pattern: /(?:^|[/\.])(?:kimi-)?k3(?:-256k)?(?:$|[-\[])/,
    capability: officialCapability(
      KIMI_K3_EFFORTS,
      "high",
      KIMI_K3_MODEL_GUIDE_URL,
    ),
  },
  {
    pattern: /(?:^|[/\.])glm-5\.2(?:$|[-\[])/,
    capability: officialCapability(
      ALL_REASONING_EFFORTS,
      "max",
      GLM_THINKING_GUIDE_URL,
    ),
  },
  {
    pattern:
      /(?:^|[/\.])claude-(?:(?:opus-(?:4-(?:7|8)|5))|(?:sonnet-5)|(?:fable-5)|(?:mythos-(?:5|preview)))(?:$|-)/,
    capability: officialCapability(
      CLAUDE_XHIGH_EFFORTS,
      "high",
      ANTHROPIC_EFFORT_GUIDE_URL,
    ),
  },
  {
    pattern: /(?:^|[/\.])claude-(?:opus|sonnet)-4-6(?:$|-)/,
    capability: officialCapability(
      CLAUDE_46_EFFORTS,
      "high",
      ANTHROPIC_EFFORT_GUIDE_URL,
    ),
  },
  {
    // Earlier Claude versions use manual `thinking: { type: "enabled",
    // budget_tokens }`, not the effort parameter exposed by this setting.
    pattern: /(?:^|[/\.])claude-(?:$|[-\d])/,
    capability: {
      status: "unsupported",
      supportedEfforts: [],
      defaultEffort: null,
      sourceUrl: ANTHROPIC_EFFORT_GUIDE_URL,
      official: true,
    },
  },
  {
    pattern: /^gpt-5\.6(?:-(?:sol|terra|luna))?(?:$|-\d{4}-\d{2}-\d{2})/,
    capability: officialCapability(
      GPT_5_6_EFFORTS,
      "medium",
      OPENAI_MODEL_GUIDE_URL,
    ),
  },
  {
    pattern: /^gpt-5\.5-pro(?:$|-\d{4}-\d{2}-\d{2})/,
    capability: officialCapability(
      ["medium", "high", "xhigh"],
      "high",
      "https://developers.openai.com/api/docs/models/gpt-5.5-pro",
    ),
  },
  {
    pattern: /^gpt-5\.4(?:-(?:mini|nano))?(?:$|-\d{4}-\d{2}-\d{2})/,
    capability: officialCapability(
      GPT_5_4_EFFORTS,
      "none",
      "https://developers.openai.com/api/docs/models/gpt-5.4",
    ),
  },
  {
    pattern: /^gpt-5\.3-codex(?:$|-\d{4}-\d{2}-\d{2})/,
    capability: officialCapability(
      GPT_5_3_CODEX_EFFORTS,
      null,
      "https://developers.openai.com/api/docs/models/gpt-5.3-codex",
    ),
  },
  {
    pattern: /^gpt-5\.2(?:$|-\d{4}-\d{2}-\d{2})/,
    capability: officialCapability(
      GPT_5_4_EFFORTS,
      "none",
      "https://developers.openai.com/api/docs/models/gpt-5.2",
    ),
  },
  {
    pattern: /^gpt-5\.1(?:$|-\d{4}-\d{2}-\d{2})/,
    capability: officialCapability(
      GPT_5_1_EFFORTS,
      "none",
      "https://developers.openai.com/api/docs/models/gpt-5.1",
    ),
  },
  {
    pattern: /^gpt-5-pro(?:$|-\d{4}-\d{2}-\d{2})/,
    capability: officialCapability(
      ["high"],
      "high",
      "https://developers.openai.com/api/docs/models/gpt-5-pro",
    ),
  },
  {
    pattern: /^gpt-5(?:-(?:mini|nano))?(?:$|-\d{4}-\d{2}-\d{2})/,
    capability: officialCapability(
      GPT_5_EFFORTS,
      "medium",
      "https://developers.openai.com/api/docs/models/gpt-5",
    ),
  },
  {
    pattern: /^(?:gpt-4\.1(?:-mini|-nano)?|gpt-4o(?:-mini)?)(?:$|-\d{4})/,
    capability: {
      status: "unsupported",
      supportedEfforts: [],
      defaultEffort: null,
      sourceUrl: "https://developers.openai.com/api/docs/models",
      official: true,
    },
  },
];

const NOT_APPLICABLE_REASONING_CAPABILITY: ModelReasoningCapability = {
  status: "unsupported",
  supportedEfforts: [],
  defaultEffort: null,
  sourceUrl: null,
  official: false,
};

const UNKNOWN_REASONING_CAPABILITY: ModelReasoningCapability = {
  status: "unknown",
  supportedEfforts: ALL_REASONING_EFFORTS,
  defaultEffort: null,
  sourceUrl: null,
  official: false,
};

function officialCapability(
  supportedEfforts: readonly ReasoningEffort[],
  defaultEffort: ReasoningEffort | null,
  sourceUrl: string,
): ModelReasoningCapability {
  return {
    status: "supported",
    supportedEfforts,
    defaultEffort,
    sourceUrl,
    official: true,
  };
}

export function findOfficialModelPreset(
  model: string,
): OfficialModelPreset | null {
  const normalizedModel = model.trim().toLocaleLowerCase();
  return (
    OFFICIAL_OPENAI_MODEL_PRESETS.find(
      (preset) => preset.model.toLocaleLowerCase() === normalizedModel,
    ) ?? null
  );
}

export function resolveModelReasoningCapability(
  providerKind: ProviderKind,
  model: string,
): ModelReasoningCapability {
  if (providerKind === "codex_app_server" || providerKind === "mock") {
    return NOT_APPLICABLE_REASONING_CAPABILITY;
  }

  const normalizedModel = model.trim().toLocaleLowerCase();
  const rule = modelCapabilityRules.find(({ pattern }) =>
    pattern.test(normalizedModel),
  );
  return rule?.capability ?? UNKNOWN_REASONING_CAPABILITY;
}

export function normalizeReasoningEffortForModel(
  providerKind: ProviderKind,
  model: string,
  reasoningEffort: ReasoningEffort | null | undefined,
): ReasoningEffort | null {
  if (!reasoningEffort) return null;
  const capability = resolveModelReasoningCapability(providerKind, model);
  if (capability.status === "unknown") return reasoningEffort;
  return capability.supportedEfforts.includes(reasoningEffort)
    ? reasoningEffort
    : null;
}

export function normalizeProviderReasoningEffort(
  provider: ProviderSettings,
): ProviderSettings {
  const reasoningEffort = normalizeReasoningEffortForModel(
    provider.kind,
    provider.model,
    provider.reasoningEffort,
  );
  return reasoningEffort === (provider.reasoningEffort ?? null)
    ? provider
    : { ...provider, reasoningEffort };
}

export function providerDisplayName(
  provider: Pick<ProviderSettings, "id" | "name">,
): string {
  const name = typeof provider.name === "string" ? provider.name.trim() : "";
  return name || provider.id;
}

export function normalizeProviderNames(
  providers: ProviderSettings[],
): ProviderSettings[] {
  return providers.map((provider) => {
    const name = providerDisplayName(provider);
    return provider.name === name ? provider : { ...provider, name };
  });
}
