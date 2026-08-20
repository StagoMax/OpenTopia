import type {
  ModelContextItem,
  TokenEstimateBreakdown,
  TokenEstimateDetail,
} from "./types";

export function emptyTokenBreakdown(): TokenEstimateBreakdown {
  return {
    baseInstructions: 0,
    developerInstructions: 0,
    repositoryInstructions: 0,
    runtimeContext: 0,
    skillInstructions: 0,
    summaries: 0,
    checkpoints: 0,
    conversation: 0,
    currentUser: 0,
    toolCalls: 0,
    toolResults: 0,
    directToolSchemas: 0,
    deferredToolCatalog: 0,
    loadedToolSchemas: 0,
    toolSchemas: 0,
    outputSchema: 0,
    turnAssistantState: 0,
    providerState: 0,
    other: 0,
    total: 0,
    details: [],
  };
}

export function addTokenBreakdown(
  target: TokenEstimateBreakdown,
  source: TokenEstimateBreakdown,
): void {
  target.baseInstructions += source.baseInstructions;
  target.developerInstructions += source.developerInstructions;
  target.repositoryInstructions += source.repositoryInstructions;
  target.runtimeContext += source.runtimeContext;
  target.skillInstructions += source.skillInstructions;
  target.summaries += source.summaries;
  target.checkpoints += source.checkpoints;
  target.conversation += source.conversation;
  target.currentUser += source.currentUser;
  target.toolCalls += source.toolCalls;
  target.toolResults += source.toolResults;
  target.directToolSchemas =
    (target.directToolSchemas ?? 0) + (source.directToolSchemas ?? 0);
  target.deferredToolCatalog =
    (target.deferredToolCatalog ?? 0) + (source.deferredToolCatalog ?? 0);
  target.loadedToolSchemas =
    (target.loadedToolSchemas ?? 0) + (source.loadedToolSchemas ?? 0);
  target.toolSchemas += source.toolSchemas;
  target.outputSchema = (target.outputSchema ?? 0) + (source.outputSchema ?? 0);
  target.turnAssistantState =
    (target.turnAssistantState ?? 0) + (source.turnAssistantState ?? 0);
  target.providerState += source.providerState;
  target.other += source.other;
  target.total += source.total;
  target.details = mergeTokenEstimateDetails(
    target.details ?? [],
    source.details ?? [],
  );
}

/**
 * Old ModelContextBuilt events contain flat totals plus the materialized
 * context items, but no attribution tree. Recover only distinctions that are
 * evidenced by those items (especially conversation roles), and make the
 * unrecoverable remainder explicit instead of inventing precision.
 */
export function hydrateLegacyTokenBreakdown(
  breakdown: TokenEstimateBreakdown,
  items: ModelContextItem[],
): TokenEstimateBreakdown {
  if ((breakdown.details?.length ?? 0) > 0) return breakdown;

  const recovered = new Map<string, TokenEstimateDetail[]>();
  for (const item of items) {
    const rootId = contextRootId(item.kind);
    if (!rootId) continue;
    const child = legacyContextChild(item);
    recovered.set(
      rootId,
      mergeTokenEstimateDetails(recovered.get(rootId) ?? [], [child]),
    );
  }

  return {
    ...breakdown,
    details: legacyTokenBreakdownDetails(breakdown).map((root) => ({
      ...root,
      children: reconcileLegacyChildren(
        root.id,
        root.tokens,
        recovered.get(root.id) ?? root.children ?? [],
      ),
    })),
  };
}

export function legacyTokenBreakdownDetails(
  breakdown: TokenEstimateBreakdown,
): TokenEstimateDetail[] {
  const toolChildren = [
    flatDetail("direct_tool_schemas", breakdown.directToolSchemas ?? 0),
    flatDetail("deferred_tool_catalog", breakdown.deferredToolCatalog ?? 0),
    flatDetail("loaded_tool_schemas", breakdown.loadedToolSchemas ?? 0),
  ].filter((item) => item.tokens > 0);
  return [
    flatDetail("base_instructions", breakdown.baseInstructions),
    flatDetail("developer_instructions", breakdown.developerInstructions),
    flatDetail("repository_instructions", breakdown.repositoryInstructions),
    flatDetail("runtime_context", breakdown.runtimeContext),
    flatDetail("skill_instructions", breakdown.skillInstructions),
    flatDetail("summaries", breakdown.summaries),
    flatDetail("checkpoints", breakdown.checkpoints),
    flatDetail("conversation", breakdown.conversation),
    flatDetail("current_user", breakdown.currentUser),
    flatDetail("tool_calls", breakdown.toolCalls),
    flatDetail("tool_results", breakdown.toolResults),
    {
      ...flatDetail("tool_schemas", breakdown.toolSchemas),
      children: toolChildren,
    },
    flatDetail("output_schema", breakdown.outputSchema ?? 0),
    flatDetail("turn_assistant_state", breakdown.turnAssistantState ?? 0),
    flatDetail("provider_state", breakdown.providerState),
    flatDetail("other", breakdown.other),
  ];
}

export function mergeTokenEstimateDetails(
  target: TokenEstimateDetail[],
  source: TokenEstimateDetail[],
): TokenEstimateDetail[] {
  if (source.length === 0) return target;

  const indexes = new Map(target.map((detail, index) => [detail.id, index]));
  for (const detail of source) {
    const index = indexes.get(detail.id);
    if (index === undefined) {
      indexes.set(detail.id, target.length);
      target.push(cloneDetail(detail));
      continue;
    }
    const existing = target[index];
    existing.tokens += detail.tokens;
    existing.children = mergeTokenEstimateDetails(
      existing.children ?? [],
      detail.children ?? [],
    );
  }
  return target;
}

function cloneDetail(detail: TokenEstimateDetail): TokenEstimateDetail {
  return {
    ...detail,
    children: detail.children?.map(cloneDetail),
  };
}

function flatDetail(id: string, tokens: number): TokenEstimateDetail {
  return { id, label: id, tokens, children: [] };
}

function contextRootId(kind: ModelContextItem["kind"]): string | null {
  switch (kind) {
    case "base_instructions":
      return "base_instructions";
    case "developer_instructions":
      return "developer_instructions";
    case "repository_instructions":
      return "repository_instructions";
    case "environment":
    case "world_state":
    case "capability_catalog":
      return "runtime_context";
    case "skill_instructions":
    case "skill":
      return "skill_instructions";
    case "summary":
      return "summaries";
    case "checkpoint":
      return "checkpoints";
    case "conversation":
      return "conversation";
    case "user":
      return "current_user";
    case "tool_call":
      return "tool_calls";
    case "tool_result":
      return "tool_results";
    default:
      return null;
  }
}

function legacyContextChild(item: ModelContextItem): TokenEstimateDetail {
  if (item.kind === "conversation") {
    const id = `${item.role}_messages`;
    return flatDetail(id, item.tokenEstimate);
  }
  if (item.kind === "user") {
    return flatDetail("message_text", item.tokenEstimate);
  }
  const source = item.source || item.kind;
  return {
    id: source,
    label: source,
    tokens: item.tokenEstimate,
    children: [],
  };
}

function reconcileLegacyChildren(
  rootId: string,
  target: number,
  source: TokenEstimateDetail[],
): TokenEstimateDetail[] {
  if (target <= 0) return [];
  const children = source.map(cloneDetail);
  const attributed = children.reduce((total, child) => total + child.tokens, 0);
  if (attributed < target) {
    const id =
      rootId === "provider_state"
        ? "legacy_unclassified_provider_state"
        : "legacy_unattributed";
    children.push(flatDetail(id, target - attributed));
    return children;
  }
  if (attributed === target) return children;

  // A legacy flat total may have used a different estimator. Scale recovered
  // evidence to the parent while preserving the exact displayed total.
  let allocated = 0;
  return children.map((child, index) => {
    const tokens =
      index === children.length - 1
        ? target - allocated
        : Math.floor((child.tokens * target) / attributed);
    allocated += tokens;
    return { ...child, tokens };
  });
}
