import assert from "node:assert/strict";
import test from "node:test";

import type * as BreakdownModule from "./usageTokenBreakdown";
import type { ModelContextItem, TokenEstimateBreakdown } from "./types";

const { addTokenBreakdown, emptyTokenBreakdown, hydrateLegacyTokenBreakdown } =
  (await import("./usageTokenBreakdown" + ".ts")) as typeof BreakdownModule;

test("aggregates nested attribution by stable detail id", () => {
  const target = emptyTokenBreakdown();
  addTokenBreakdown(target, breakdown(10, 3));
  addTokenBreakdown(target, breakdown(20, 7));

  assert.equal(target.total, 40);
  assert.equal(target.outputSchema, 10);
  assert.deepEqual(target.details, [
    {
      id: "base_instructions",
      label: "Base instructions",
      tokens: 30,
      children: [
        {
          id: "identity_and_objective",
          label: "Identity and objective",
          tokens: 30,
          children: [],
        },
      ],
    },
  ]);
});

test("recovers legacy conversation roles and exposes the unrecoverable remainder", () => {
  const legacy = breakdown(0, 0);
  legacy.conversation = 30;
  legacy.providerState = 7;
  legacy.total = 37;
  legacy.details = [];

  const hydrated = hydrateLegacyTokenBreakdown(legacy, [
    legacyContextItem("conversation:user", "user", 10),
    legacyContextItem("conversation:assistant", "assistant", 15),
  ]);
  const conversation = hydrated.details?.find(
    (detail) => detail.id === "conversation",
  );
  const providerState = hydrated.details?.find(
    (detail) => detail.id === "provider_state",
  );

  assert.deepEqual(conversation?.children, [
    { id: "user_messages", label: "user_messages", tokens: 10, children: [] },
    {
      id: "assistant_messages",
      label: "assistant_messages",
      tokens: 15,
      children: [],
    },
    {
      id: "legacy_unattributed",
      label: "legacy_unattributed",
      tokens: 5,
      children: [],
    },
  ]);
  assert.deepEqual(providerState?.children, [
    {
      id: "legacy_unclassified_provider_state",
      label: "legacy_unclassified_provider_state",
      tokens: 7,
      children: [],
    },
  ]);
  assert.equal(hydrated.total, 37);
});

function breakdown(
  baseTokens: number,
  outputSchema: number,
): TokenEstimateBreakdown {
  return {
    baseInstructions: baseTokens,
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
    toolSchemas: 0,
    outputSchema,
    providerState: 0,
    other: 0,
    total: baseTokens + outputSchema,
    details: [
      {
        id: "base_instructions",
        label: "Base instructions",
        tokens: baseTokens,
        children: [
          {
            id: "identity_and_objective",
            label: "Identity and objective",
            tokens: baseTokens,
          },
        ],
      },
    ],
  };
}

function legacyContextItem(
  source: string,
  role: ModelContextItem["role"],
  tokenEstimate: number,
): ModelContextItem {
  return {
    id: source,
    kind: "conversation",
    role,
    authority: role,
    lifecycle: "thread",
    source,
    content: [{ type: "text", text: source }],
    contentHash: source,
    tokenEstimate,
    cacheScope: "thread",
    sensitivity: "workspace",
  };
}
