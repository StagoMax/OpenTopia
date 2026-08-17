import assert from "node:assert/strict";
import test from "node:test";

import type * as PreferencesModule from "./workbenchPreferences";

const {
  parseDraftModelSelection,
  parseLastActiveThreadIds,
  parseSidebarNavigationState,
  readDraftModelSelection,
  resolveDraftModelSelection,
  writeDraftModelSelection,
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

test("accepts complete stored draft model selections and drops legacy protocol overrides", () => {
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
  assert.deepEqual(
    parseDraftModelSelection({
      connectionId: "tokenhub",
      modelId: "gpt-5.6-sol",
      adapter: "open_ai_responses",
      reasoningEffort: "high",
    }),
    {
      connectionId: "tokenhub",
      modelId: "gpt-5.6-sol",
      reasoningEffort: "high",
    },
  );
  assert.deepEqual(
    parseDraftModelSelection({
      connectionId: "tokenhub",
      modelId: "gpt-5.6-sol",
      adapter: "tokenhub_magic",
      reasoningEffort: "high",
    }),
    {
      connectionId: "tokenhub",
      modelId: "gpt-5.6-sol",
      reasoningEffort: "high",
    },
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

test("persists the complete last-used model state for the next task", () => {
  const stored = new Map<string, string>();
  const originalWindow = globalThis.window;
  Object.defineProperty(globalThis, "window", {
    configurable: true,
    value: {
      localStorage: {
        getItem(key: string) {
          return stored.get(key) ?? null;
        },
        removeItem(key: string) {
          stored.delete(key);
        },
        setItem(key: string, value: string) {
          stored.set(key, value);
        },
      },
    },
  });

  try {
    const selection = {
      connectionId: "openai",
      modelId: "gpt-5.6-sol",
      reasoningEffort: "xhigh" as const,
    };
    writeDraftModelSelection(selection);

    assert.deepEqual(readDraftModelSelection(), selection);
  } finally {
    if (originalWindow) {
      Object.defineProperty(globalThis, "window", {
        configurable: true,
        value: originalWindow,
      });
    } else {
      Reflect.deleteProperty(globalThis, "window");
    }
  }
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
