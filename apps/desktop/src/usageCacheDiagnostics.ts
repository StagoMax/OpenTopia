import type {
  ModelContextItem,
  ProviderCacheTrace,
  ProviderCacheTraceSegment,
} from "./types";

export type CacheReuseState = "reused" | "degraded" | "broken" | "unverified";

export type CacheBreakReason =
  | "wire_prefix_changed"
  | "content_changed"
  | "tool_catalog_changed"
  | "system_prompt_changed"
  | "cache_key_changed"
  | "request_configuration_changed"
  | "model_changed"
  | "provider_changed"
  | "input_below_minimum"
  | "stateful_context"
  | "operational_miss"
  | "diagnostic_data_missing"
  | "cache_hit"
  | "no_baseline"
  | "usage_unreported";

export type CacheBreakPoint = {
  kind:
    | ModelContextItem["kind"]
    | ProviderCacheTraceSegment["kind"]
    | "tool_catalog"
    | "system_prompt"
    | "cache_key"
    | "request_configuration"
    | "model"
    | "provider"
    | "input";
  source: string;
  cacheScope: ModelContextItem["cacheScope"] | null;
  change: "changed" | "inserted" | "removed" | "configuration";
  tokenOffsetEstimate: number | null;
  previousTokenEstimate: number | null;
  currentTokenEstimate: number | null;
  name?: string | null;
  affectedSegmentCount?: number;
  anchorSource?: string | null;
};

export type CacheReuseDiagnostic = {
  state: CacheReuseState;
  reason: CacheBreakReason;
  confidence: "high" | "medium" | "low";
  previousRequestId: string | null;
  previousCachedInputTokens: number | null;
  currentCachedInputTokens: number;
  lostCachedTokens: number;
  breakpoint: CacheBreakPoint | null;
};

export type CacheRequestObservation = {
  contextHash: string | null;
  contextItems: ModelContextItem[];
  promptCacheKey: string | null;
  previousResponseId: string | null;
  systemPrompt: string | null;
  toolCatalogSignature: string | null;
  providerCacheTrace: ProviderCacheTrace | null;
};

type CacheComparableCall = {
  id: string;
  providerId: string | null;
  model: string | null;
  adapter: string;
  inputTokens: number;
  cachedInputTokens: number;
  cacheReadTokensReported: boolean;
  cacheObservation: CacheRequestObservation;
};

export function emptyCacheDiagnostic(): CacheReuseDiagnostic {
  return {
    state: "unverified",
    reason: "usage_unreported",
    confidence: "low",
    previousRequestId: null,
    previousCachedInputTokens: null,
    currentCachedInputTokens: 0,
    lostCachedTokens: 0,
    breakpoint: null,
  };
}

export function diagnoseCacheReuse(
  current: CacheComparableCall,
  previous: CacheComparableCall | null,
): CacheReuseDiagnostic {
  const base = {
    previousRequestId: previous?.id ?? null,
    previousCachedInputTokens: previous?.cachedInputTokens ?? null,
    currentCachedInputTokens: current.cachedInputTokens,
    lostCachedTokens: 0,
    breakpoint: null,
  } satisfies Pick<
    CacheReuseDiagnostic,
    | "previousRequestId"
    | "previousCachedInputTokens"
    | "currentCachedInputTokens"
    | "lostCachedTokens"
    | "breakpoint"
  >;

  if (!current.cacheReadTokensReported) {
    return {
      ...base,
      state: "unverified",
      reason: "usage_unreported",
      confidence: "low",
    };
  }
  if (!previous || previous.cachedInputTokens === 0) {
    return {
      ...base,
      state: current.cachedInputTokens > 0 ? "reused" : "unverified",
      reason: current.cachedInputTokens > 0 ? "cache_hit" : "no_baseline",
      confidence: current.cachedInputTokens > 0 ? "high" : "low",
    };
  }

  const expectedReusableTokens = Math.min(
    previous.cachedInputTokens,
    current.inputTokens,
  );
  const lostCachedTokens = Math.max(
    0,
    expectedReusableTokens - current.cachedInputTokens,
  );
  if (lostCachedTokens === 0) {
    return {
      ...base,
      state: "reused",
      reason: "cache_hit",
      confidence: "high",
    };
  }

  const state = current.cachedInputTokens === 0 ? "broken" : "degraded";
  const shared = { ...base, state, lostCachedTokens } as const;
  if (isOpenAiCall(current) && current.inputTokens < 1_024) {
    return {
      ...shared,
      reason: "input_below_minimum",
      confidence: "high",
      breakpoint: configurationBreakpoint("input", "input_tokens"),
    };
  }
  if (changedKnownValue(previous.providerId, current.providerId)) {
    return {
      ...shared,
      reason: "provider_changed",
      confidence: "high",
      breakpoint: configurationBreakpoint(
        "provider",
        current.providerId ?? "provider",
      ),
    };
  }
  if (changedKnownValue(previous.model, current.model)) {
    return {
      ...shared,
      reason: "model_changed",
      confidence: "high",
      breakpoint: configurationBreakpoint("model", current.model ?? "model"),
    };
  }

  const previousTrace = previous.cacheObservation.providerCacheTrace;
  const currentTrace = current.cacheObservation.providerCacheTrace;
  if (previousTrace && currentTrace) {
    if (
      changedOptionalValue(
        previousTrace.promptCacheKeyHash ?? null,
        currentTrace.promptCacheKeyHash ?? null,
      )
    ) {
      return {
        ...shared,
        reason: "cache_key_changed",
        confidence: "high",
        breakpoint: configurationBreakpoint("cache_key", "prompt_cache_key"),
      };
    }
    if (
      changedOptionalValue(
        previousTrace.toolCatalogHash ?? null,
        currentTrace.toolCatalogHash ?? null,
      )
    ) {
      return {
        ...shared,
        reason: "tool_catalog_changed",
        confidence: "high",
        breakpoint: configurationBreakpoint("tool_catalog", "tools"),
      };
    }
    const configurationChange = findConfigurationChange(
      previousTrace,
      currentTrace,
    );
    if (configurationChange) {
      return {
        ...shared,
        reason: "request_configuration_changed",
        confidence: "high",
        breakpoint: configurationChange,
      };
    }
    const wireBreakpoint = findProviderBreakpoint(
      previousTrace.segments,
      currentTrace.segments,
    );
    if (wireBreakpoint) {
      return {
        ...shared,
        reason: "wire_prefix_changed",
        confidence: "high",
        breakpoint: wireBreakpoint,
      };
    }
  }

  if (
    previous.cacheObservation.promptCacheKey !==
      current.cacheObservation.promptCacheKey &&
    (previous.cacheObservation.promptCacheKey !== null ||
      current.cacheObservation.promptCacheKey !== null)
  ) {
    return {
      ...shared,
      reason: "cache_key_changed",
      confidence: "high",
      breakpoint: configurationBreakpoint("cache_key", "prompt_cache_key"),
    };
  }
  if (
    changedKnownValue(
      previous.cacheObservation.toolCatalogSignature,
      current.cacheObservation.toolCatalogSignature,
    )
  ) {
    return {
      ...shared,
      reason: "tool_catalog_changed",
      confidence: "high",
      breakpoint: configurationBreakpoint("tool_catalog", "tool_candidates"),
    };
  }
  const contextBreakpoint = findContextBreakpoint(
    previous.cacheObservation.contextItems,
    current.cacheObservation.contextItems,
  );
  if (contextBreakpoint) {
    return {
      ...shared,
      reason: "content_changed",
      confidence: "medium",
      breakpoint: contextBreakpoint,
    };
  }
  if (
    changedKnownValue(
      previous.cacheObservation.systemPrompt,
      current.cacheObservation.systemPrompt,
    )
  ) {
    return {
      ...shared,
      reason: "system_prompt_changed",
      confidence: "high",
      breakpoint: configurationBreakpoint("system_prompt", "system_prompt"),
    };
  }
  if (
    previousTrace?.previousResponseIdPresent ||
    currentTrace?.previousResponseIdPresent ||
    previous.cacheObservation.previousResponseId ||
    current.cacheObservation.previousResponseId
  ) {
    return {
      ...shared,
      reason: "stateful_context",
      confidence: "low",
    };
  }
  if (!previousTrace || !currentTrace) {
    if (
      previous.cacheObservation.contextItems.length > 0 &&
      current.cacheObservation.contextItems.length > 0
    ) {
      return {
        ...shared,
        reason: "operational_miss",
        confidence: "low",
      };
    }
    return {
      ...shared,
      reason: "diagnostic_data_missing",
      confidence: "low",
    };
  }
  return {
    ...shared,
    reason: "operational_miss",
    confidence: "medium",
  };
}

function findProviderBreakpoint(
  previous: ProviderCacheTraceSegment[],
  current: ProviderCacheTraceSegment[],
): CacheBreakPoint | null {
  if (previous.length === 0 && current.length === 0) return null;
  if (previous.length === 0) {
    return providerSegmentBreakpoint(
      current[0],
      "inserted",
      0,
      null,
      current[0]?.tokenEstimate ?? null,
      current.length,
      null,
    );
  }
  if (current.length === 0) {
    return providerSegmentBreakpoint(
      previous[0],
      "removed",
      0,
      previous[0]?.tokenEstimate ?? null,
      null,
      previous.length,
      null,
    );
  }
  let index = 0;
  while (
    index < previous.length &&
    index < current.length &&
    providerSegmentMatches(previous[index], current[index])
  ) {
    index += 1;
  }
  if (index === previous.length) return null;

  const previousItem = previous[index];
  const currentItem = current[index];
  const tokenOffsetEstimate = current
    .slice(0, index)
    .reduce((total, item) => total + item.tokenEstimate, 0);
  if (!currentItem) {
    return providerSegmentBreakpoint(
      previousItem,
      "removed",
      tokenOffsetEstimate,
      previousItem?.tokenEstimate ?? null,
      null,
      previous.length - index,
      null,
    );
  }

  const previousAnchorInCurrent = previousItem
    ? current.findIndex(
        (item, candidateIndex) =>
          candidateIndex > index && providerSegmentMatches(previousItem, item),
      )
    : -1;
  if (previousAnchorInCurrent > index) {
    return providerSegmentBreakpoint(
      currentItem,
      "inserted",
      tokenOffsetEstimate,
      null,
      currentItem.tokenEstimate,
      previousAnchorInCurrent - index,
      current[previousAnchorInCurrent]?.source ?? null,
    );
  }

  const currentAnchorInPrevious = previous.findIndex(
    (item, candidateIndex) =>
      candidateIndex > index && providerSegmentMatches(item, currentItem),
  );
  if (currentAnchorInPrevious > index) {
    return providerSegmentBreakpoint(
      previousItem,
      "removed",
      tokenOffsetEstimate,
      previousItem?.tokenEstimate ?? null,
      null,
      currentAnchorInPrevious - index,
      currentItem.source,
    );
  }

  return providerSegmentBreakpoint(
    currentItem,
    "changed",
    tokenOffsetEstimate,
    previousItem?.tokenEstimate ?? null,
    currentItem.tokenEstimate,
    1,
    null,
  );
}

function providerSegmentMatches(
  left: ProviderCacheTraceSegment | undefined,
  right: ProviderCacheTraceSegment | undefined,
): boolean {
  return Boolean(
    left &&
    right &&
    left.kind === right.kind &&
    (left.name ?? null) === (right.name ?? null) &&
    left.contentHash === right.contentHash,
  );
}

function providerSegmentBreakpoint(
  item: ProviderCacheTraceSegment | undefined,
  change: CacheBreakPoint["change"],
  tokenOffsetEstimate: number,
  previousTokenEstimate: number | null,
  currentTokenEstimate: number | null,
  affectedSegmentCount: number,
  anchorSource: string | null,
): CacheBreakPoint | null {
  if (!item) return null;
  return {
    kind: item.kind,
    source: item.source,
    cacheScope: null,
    change,
    tokenOffsetEstimate,
    previousTokenEstimate,
    currentTokenEstimate,
    name: item.name ?? null,
    affectedSegmentCount,
    anchorSource,
  };
}

function findConfigurationChange(
  previous: ProviderCacheTrace,
  current: ProviderCacheTrace,
): CacheBreakPoint | null {
  const previousValues = new Map(
    (previous.configuration ?? []).map((property) => [
      property.name,
      property.valueHash,
    ]),
  );
  const currentValues = new Map(
    (current.configuration ?? []).map((property) => [
      property.name,
      property.valueHash,
    ]),
  );
  const names = [
    ...new Set([...previousValues.keys(), ...currentValues.keys()]),
  ].sort();
  for (const name of names) {
    const before = previousValues.get(name);
    const after = currentValues.get(name);
    if (before === after) continue;
    return {
      ...configurationBreakpoint("request_configuration", name),
      change:
        before === undefined
          ? "inserted"
          : after === undefined
            ? "removed"
            : "changed",
    };
  }
  return null;
}

function findContextBreakpoint(
  previous: ModelContextItem[],
  current: ModelContextItem[],
): CacheBreakPoint | null {
  if (previous.length === 0 || current.length === 0) return null;
  let index = 0;
  while (
    index < previous.length &&
    index < current.length &&
    contextItemMatches(previous[index], current[index])
  ) {
    index += 1;
  }
  if (index === previous.length) return null;

  const previousItem = previous[index];
  const currentItem = current[index];
  const tokenOffsetEstimate = current
    .slice(0, index)
    .reduce((total, item) => total + item.tokenEstimate, 0);
  if (!currentItem) {
    return contextItemBreakpoint(
      previousItem,
      "removed",
      tokenOffsetEstimate,
      previousItem?.tokenEstimate ?? null,
      null,
    );
  }
  if (
    current[index + 1] &&
    previousItem &&
    contextItemMatches(previousItem, current[index + 1])
  ) {
    return contextItemBreakpoint(
      currentItem,
      "inserted",
      tokenOffsetEstimate,
      null,
      currentItem.tokenEstimate,
    );
  }
  if (
    previous[index + 1] &&
    contextItemMatches(previous[index + 1], currentItem)
  ) {
    return contextItemBreakpoint(
      previousItem,
      "removed",
      tokenOffsetEstimate,
      previousItem?.tokenEstimate ?? null,
      null,
    );
  }
  return contextItemBreakpoint(
    currentItem,
    "changed",
    tokenOffsetEstimate,
    previousItem?.tokenEstimate ?? null,
    currentItem.tokenEstimate,
  );
}

function contextItemMatches(
  left: ModelContextItem | undefined,
  right: ModelContextItem | undefined,
): boolean {
  return Boolean(
    left &&
    right &&
    left.kind === right.kind &&
    left.role === right.role &&
    left.authority === right.authority &&
    left.source === right.source &&
    left.cacheScope === right.cacheScope &&
    left.contentHash === right.contentHash,
  );
}

function contextItemBreakpoint(
  item: ModelContextItem | undefined,
  change: CacheBreakPoint["change"],
  tokenOffsetEstimate: number,
  previousTokenEstimate: number | null,
  currentTokenEstimate: number | null,
): CacheBreakPoint | null {
  if (!item) return null;
  return {
    kind: item.kind,
    source: item.source,
    cacheScope: item.cacheScope,
    change,
    tokenOffsetEstimate,
    previousTokenEstimate,
    currentTokenEstimate,
  };
}

function configurationBreakpoint(
  kind: Extract<
    CacheBreakPoint["kind"],
    | "tool_catalog"
    | "system_prompt"
    | "cache_key"
    | "request_configuration"
    | "model"
    | "provider"
    | "input"
  >,
  source: string,
): CacheBreakPoint {
  return {
    kind,
    source,
    cacheScope: null,
    change: "configuration",
    tokenOffsetEstimate: null,
    previousTokenEstimate: null,
    currentTokenEstimate: null,
  };
}

function changedKnownValue(
  previous: string | null,
  current: string | null,
): boolean {
  return previous !== null && current !== null && previous !== current;
}

function changedOptionalValue(
  previous: string | null,
  current: string | null,
): boolean {
  return previous !== current && (previous !== null || current !== null);
}

function isOpenAiCall(call: CacheComparableCall): boolean {
  return call.adapter.toLowerCase().includes("openai");
}
