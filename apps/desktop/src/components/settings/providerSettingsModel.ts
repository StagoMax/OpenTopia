import type { ProviderImportDraft } from "../../providerImport";
import type {
  AppSettings,
  ProviderAdapterKind,
  ProviderAuthKind,
  ProviderKind,
  ProviderSettings,
  ProviderTransportKind,
} from "../../types";

export type ModelDiscoveryState =
  | { status: "idle" }
  | { status: "discovering" }
  | { status: "success"; modelCount: number }
  | { status: "error"; message: string };

const providerDiscoveryDependentFields = new Set<keyof ProviderSettings>([
  "baseUrl",
  "model",
  "kind",
  "transport",
  "auth",
  "allowedAdapters",
  "preferredAdapter",
]);

export type ProviderBaseUrlPreset = {
  id: string;
  label: string;
  baseUrl: string;
  kind: Extract<ProviderKind, "openai_compatible" | "anthropic">;
};

export const providerBaseUrlPresets: readonly ProviderBaseUrlPreset[] = [
  {
    id: "openai",
    label: "OpenAI",
    baseUrl: "https://api.openai.com/v1",
    kind: "openai_compatible",
  },
  {
    id: "anthropic",
    label: "Anthropic",
    baseUrl: "https://api.anthropic.com",
    kind: "anthropic",
  },
  {
    id: "openrouter",
    label: "OpenRouter",
    baseUrl: "https://openrouter.ai/api/v1",
    kind: "openai_compatible",
  },
  {
    id: "deepseek",
    label: "DeepSeek",
    baseUrl: "https://api.deepseek.com/v1",
    kind: "openai_compatible",
  },
  {
    id: "qwen-china",
    label: "Qwen / DashScope (CN)",
    baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    kind: "openai_compatible",
  },
  {
    id: "qwen-international",
    label: "Qwen / DashScope (GLOBAL)",
    baseUrl: "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
    kind: "openai_compatible",
  },
  {
    id: "moonshot-china",
    label: "Moonshot / Kimi (CN)",
    baseUrl: "https://api.moonshot.cn/v1",
    kind: "openai_compatible",
  },
  {
    id: "moonshot-international",
    label: "Moonshot / Kimi (GLOBAL)",
    baseUrl: "https://api.moonshot.ai/v1",
    kind: "openai_compatible",
  },
  {
    id: "zhipu-china",
    label: "Zhipu AI / GLM (CN)",
    baseUrl: "https://open.bigmodel.cn/api/paas/v4",
    kind: "openai_compatible",
  },
  {
    id: "zhipu-international",
    label: "Z.AI / GLM (GLOBAL)",
    baseUrl: "https://api.z.ai/api/paas/v4",
    kind: "openai_compatible",
  },
  {
    id: "ollama",
    label: "Ollama (local)",
    baseUrl: "http://localhost:11434/v1",
    kind: "openai_compatible",
  },
];

export type ProviderProtocolSelection =
  "openai_auto" | "open_ai_chat" | "open_ai_responses" | "anthropic_messages";

export function providerAxesFromKind(kind: ProviderKind): {
  transport: ProviderTransportKind;
  auth: ProviderAuthKind;
  allowedAdapters: ProviderAdapterKind[];
  preferredAdapter: ProviderAdapterKind | null;
} {
  if (kind === "codex_app_server") {
    return {
      transport: "codex_app_server",
      auth: "codex_session",
      allowedAdapters: ["codex_app_server"],
      preferredAdapter: "codex_app_server",
    };
  }
  if (kind === "mock") {
    return {
      transport: "mock",
      auth: "none",
      allowedAdapters: ["mock"],
      preferredAdapter: "mock",
    };
  }
  if (kind === "anthropic") {
    return {
      transport: "http",
      auth: "x_api_key",
      allowedAdapters: ["anthropic_messages"],
      preferredAdapter: "anthropic_messages",
    };
  }
  return {
    transport: "http",
    auth: "bearer",
    allowedAdapters: ["open_ai_responses", "open_ai_chat"],
    preferredAdapter: kind === "openai_responses" ? "open_ai_responses" : null,
  };
}

export function providerEffectiveTransport(
  provider: ProviderSettings,
): ProviderTransportKind {
  return provider.transport ?? providerAxesFromKind(provider.kind).transport;
}

export function providerEffectiveAuth(
  provider: ProviderSettings,
): ProviderAuthKind {
  return provider.auth ?? providerAxesFromKind(provider.kind).auth;
}

export function providerAllowedAdapters(
  provider: ProviderSettings,
): ProviderAdapterKind[] {
  return provider.allowedAdapters?.length
    ? provider.allowedAdapters
    : providerAxesFromKind(provider.kind).allowedAdapters;
}

export function providerProtocolSelection(
  provider: ProviderSettings,
): ProviderProtocolSelection {
  const allowed = providerAllowedAdapters(provider);
  if (allowed.includes("anthropic_messages")) return "anthropic_messages";
  if (
    allowed.includes("open_ai_chat") &&
    allowed.includes("open_ai_responses")
  ) {
    return "openai_auto";
  }
  return allowed.includes("open_ai_responses")
    ? "open_ai_responses"
    : "open_ai_chat";
}

/**
 * Generic manual values. K/M context labels differ between model providers,
 * so each option exposes the exact token count it stores.
 */
export const contextWindowPresets = [
  { tokens: 8_000, label: "8K（8,000 tokens）" },
  { tokens: 32_000, label: "32K（32,000 tokens）" },
  { tokens: 64_000, label: "64K（64,000 tokens）" },
  { tokens: 128_000, label: "128K（128,000 tokens）" },
  { tokens: 200_000, label: "200K（200,000 tokens）" },
  { tokens: 256_000, label: "256K（256,000 tokens）" },
  { tokens: 400_000, label: "400K（400,000 tokens）" },
  { tokens: 1_048_576, label: "1M（1,048,576 tokens）" },
  { tokens: 1_050_000, label: "1.05M（1,050,000 tokens）" },
] as const;

/**
 * Context limits are whole token counts, but providers do not consistently
 * publish limits aligned to KiB-sized increments. Keep the manual field
 * compatible with every advertised preset.
 */
export const contextWindowInputConstraints = {
  min: 4_096,
  step: 1,
} as const;

export const providerBaseUrlPresetOptions = providerBaseUrlPresets.map(
  (preset) => ({
    value: preset.id,
    label: preset.label,
  }),
);

export const contextWindowPresetOptions = [
  { value: "auto", label: "自动识别" },
  ...contextWindowPresets.map((preset) => ({
    value: preset.tokens.toString(),
    label: preset.label,
  })),
];

export function providerDiscoverySignature(
  provider: ProviderSettings,
  pendingApiKey: string,
): string | null {
  if (
    providerEffectiveTransport(provider) !== "http" ||
    !isHttpUrl(provider.baseUrl)
  ) {
    return null;
  }
  const auth = providerEffectiveAuth(provider);
  if (auth !== "none" && !pendingApiKey && !provider.apiKeyConfigured) {
    return null;
  }
  return `${provider.id}\u0000${provider.baseUrl.trim()}\u0000${provider.model.trim()}\u0000${providerProtocolSelection(provider)}\u0000${auth}\u0000${
    pendingApiKey || "configured"
  }`;
}

/** A completed model sync is persisted and should be reused when revisiting settings. */
export function hasCachedProviderModelCatalog(
  provider: ProviderSettings,
): boolean {
  return provider.modelsSyncedAt != null;
}

/** Changes to these fields require a fresh catalog and adapter negotiation. */
export function providerChangeInvalidatesModelDiscovery(
  field: keyof ProviderSettings,
): boolean {
  return providerDiscoveryDependentFields.has(field);
}

export function isHttpUrl(value: string): boolean {
  try {
    const url = new URL(value.trim());
    return url.protocol === "http:" || url.protocol === "https:";
  } catch {
    return false;
  }
}

export function createProviderSettings(
  id: string,
  overrides: Partial<ProviderSettings> = {},
): ProviderSettings {
  const axes = providerAxesFromKind(overrides.kind ?? "openai_compatible");
  return {
    id,
    name: "",
    kind: "openai_compatible",
    ...axes,
    baseUrl: "",
    model: "",
    enabledFamilies: [],
    syncedModels: [],
    modelsSyncedAt: null,
    temperature: null,
    maxOutputTokens: null,
    contextWindowTokens: null,
    reasoningEffort: null,
    storeResponses: false,
    parallelToolCalls: true,
    promptCacheKey: null,
    promptCachePolicy: null,
    responsesCompactionThresholdTokens: null,
    rolloutBudget: null,
    openaiCompatibility: null,
    apiKeySource: "OPENTOPIA_API_KEY",
    apiKeyConfigured: false,
    healthStatus: null,
    ...overrides,
  };
}

export function uniqueProviderId(
  suggestedId: string,
  providers: ProviderSettings[],
): string {
  const base =
    suggestedId
      .trim()
      .toLocaleLowerCase()
      .replace(/[^a-z0-9._-]+/g, "-")
      .replace(/^-+|-+$/g, "") || "custom-provider";
  const ids = new Set(providers.map((provider) => provider.id));
  if (!ids.has(base)) return base;
  let suffix = 2;
  while (ids.has(`${base}-${suffix}`)) suffix += 1;
  return `${base}-${suffix}`;
}

export function providerSettingsSnapshot(
  providers: ProviderSettings[],
  activeProviderId: string,
): string {
  return JSON.stringify({
    providers,
    activeProviderId,
  });
}

export function controlledSandboxSettings(
  sandbox: AppSettings["sandbox"],
): AppSettings["sandbox"] {
  return {
    ...sandbox,
    sandboxMode:
      sandbox.sandboxMode === "danger-full-access"
        ? "workspace-write"
        : sandbox.sandboxMode,
    enforcement:
      sandbox.enforcement === "disabled" ? "enforce" : sandbox.enforcement,
    network: sandbox.network,
  };
}

export function providerKindLabel(kind: ProviderKind): string {
  if (kind === "codex_app_server") return "Codex App Server";
  if (kind === "anthropic") return "Anthropic Messages";
  if (isOpenAiProviderKind(kind)) return "OpenAI Compatible";
  return "Mock";
}

export function providerProtocolDescription(
  provider: ProviderSettings,
): string {
  if (providerEffectiveTransport(provider) === "codex_app_server") {
    return "使用本机 Codex App Server，不需要 URL 或 API 密钥。";
  }
  if (providerEffectiveTransport(provider) === "mock") {
    return "仅用于本地模拟，不会调用远程 API。";
  }
  if (providerProtocolSelection(provider) === "openai_auto") {
    return "本连接会自动选择 Chat Completions 或 Responses，并应用到所有使用它的任务。";
  }
  if (providerProtocolSelection(provider) === "anthropic_messages") {
    return "本连接的所有任务使用 Anthropic Messages 线协议；认证方式由上方设置独立决定。";
  }
  return "仅启用所选协议，并应用到所有使用本连接的任务；连接测试会保存该模型的能力档案。";
}

export function isOpenAiProviderKind(kind: ProviderKind): boolean {
  return kind === "openai_compatible" || kind === "openai_responses";
}

export function providerFeatureSupportLabel(
  support: "supported" | "unsupported" | "unknown",
): string {
  if (support === "supported") return "支持";
  if (support === "unsupported") return "不支持";
  return "未确认";
}

export function providerHealthStatusLabel(status?: string): string {
  if (status === "ready") return "可用于对话";
  if (status === "needs_negotiation") return "等待能力检测";
  if (status === "mock_or_unconfigured") return "未配置";
  if (status === "local_codex") return "本地可用";
  if (status === "configured") return "已配置";
  return status ?? "未检测";
}

export function formatImportFormat(
  format: ProviderImportDraft["detectedFormat"],
): string {
  if (format === "env") return "环境变量";
  if (format === "curl") return "curl";
  if (format === "json") return "JSON";
  return "预设";
}
