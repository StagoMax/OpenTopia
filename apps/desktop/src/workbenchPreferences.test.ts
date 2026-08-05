import assert from "node:assert/strict";
import test from "node:test";

import type * as PreferencesModule from "./workbenchPreferences";

const {
  parseDraftModelSelection,
  parseLastActiveThreadIds,
  parseSidebarNavigationState,
  resolveDraftModelSelection,
} = (await import(
  "./workbenchPreferences" + ".ts"
)) as typeof PreferencesModule;

test("restores durable sidebar navigation state and removes invalid ids", () => {
  assert.deepEqual(
    parseSidebarNavigationState({
      expandedProjectIds: ["project-a", "project-a", 3, "project-b"],
      unassignedExpanded: true,
      archivedExpanded: false,
      collapsed: true,
    }),
    {
      expandedProjectIds: ["project-a", "project-b"],
      unassignedExpanded: true,
      archivedExpanded: false,
      collapsed: true,
    },
  );
});

test("accepts only complete stored draft model selections", () => {
  assert.deepEqual(
    parseDraftModelSelection({
      connectionId: "openai",
      modelId: "gpt-5.6-sol",
      reasoningEffort: "high",
    }),
    {
      connectionId: "openai",
      modelId: "gpt-5.6-sol",
      reasoningEffort: "high",
    },
  );
  assert.equal(
    parseDraftModelSelection({
      connectionId: "openai",
      modelId: "gpt-5.6-sol",
      reasoningEffort: "extreme",
    }),
    null,
  );
});

test("restores the chosen new-task model and reasoning effort", () => {
  const providers = [
    {
      id: "openai",
      kind: "openai_responses" as const,
      model: "gpt-5.6-luna",
      enabledFamilies: ["gpt"],
      syncedModels: ["gpt-5.6-luna", "gpt-5.6-sol"],
      reasoningEffort: "medium" as const,
    },
  ];
  assert.deepEqual(
    resolveDraftModelSelection(providers, "openai", {
      connectionId: "openai",
      modelId: "gpt-5.6-sol",
      reasoningEffort: "high",
    }),
    {
      connectionId: "openai",
      modelId: "gpt-5.6-sol",
      reasoningEffort: "high",
    },
  );
});

test("falls back only when a stored connection or model is no longer usable", () => {
  assert.deepEqual(
    resolveDraftModelSelection(
      [
        {
          id: "openai",
          kind: "openai_responses",
          model: "gpt-5.6-luna",
          enabledFamilies: ["gpt"],
          syncedModels: ["gpt-5.6-luna"],
          reasoningEffort: "medium",
        },
      ],
      "openai",
      {
        connectionId: "removed-connection",
        modelId: "removed-model",
        reasoningEffort: "high",
      },
    ),
    {
      connectionId: "openai",
      modelId: "gpt-5.6-luna",
      reasoningEffort: "medium",
    },
  );
});

test("follows an active connection changed from settings", () => {
  assert.deepEqual(
    resolveDraftModelSelection(
      [
        {
          id: "openai",
          kind: "openai_responses",
          model: "gpt-5.6-luna",
          enabledFamilies: ["gpt"],
          syncedModels: ["gpt-5.6-luna", "gpt-5.6-sol"],
          reasoningEffort: "medium",
        },
        {
          id: "local",
          kind: "openai_compatible",
          model: "qwen3-coder",
          enabledFamilies: ["qwen"],
          syncedModels: ["qwen3-coder"],
          reasoningEffort: "high",
        },
      ],
      "local",
      {
        connectionId: "openai",
        modelId: "gpt-5.6-sol",
        reasoningEffort: "high",
      },
    ),
    {
      connectionId: "local",
      modelId: "qwen3-coder",
      reasoningEffort: "high",
    },
  );
});

test("keeps the last active task independently for each experience mode", () => {
  assert.deepEqual(
    parseLastActiveThreadIds({
      work: "thread-work",
      code: "thread-code",
      flow: null,
      invalid: "thread-invalid",
    }),
    { work: "thread-work", code: "thread-code" },
  );
});
