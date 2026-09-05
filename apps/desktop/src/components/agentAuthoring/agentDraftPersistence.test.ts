import assert from "node:assert/strict";
import test from "node:test";
import type { AgentDraftForm } from "./agentDraftForm.ts";
import {
  agentDraftStorageKey,
  clearAgentDraft,
  parseStoredDraft,
  readAgentDraft,
  writeAgentDraft,
} from "./agentDraftPersistence.ts";

test("normalizes a workspace into a stable Agent draft key", () => {
  assert.equal(
    agentDraftStorageKey("J:\\Project\\OpenTopia\\"),
    "workspace:J:/Project/OpenTopia",
  );
  assert.equal(agentDraftStorageKey(null), "workspace:unassigned");
});

test("parses a stored Agent draft and rejects malformed bindings", () => {
  const fallback = draftForm();
  const parsed = parseStoredDraft(
    {
      form: {
        templateId: "audit-agent",
        name: "Audit Agent",
        connectionBindings: [
          {
            connectionId: "connection-1",
            capabilityRevision: 2,
            operationGrants: [
              { operationId: "search" },
              { operationId: "search" },
            ],
          },
        ],
      },
      updatedAt: 42,
    },
    fallback,
  );

  assert.equal(parsed?.form.name, "Audit Agent");
  assert.equal(parsed?.form.instructions, fallback.instructions);
  assert.deepEqual(parsed?.form.connectionBindings, [
    {
      connectionId: "connection-1",
      capabilityRevision: 2,
      operationGrants: [{ operationId: "search" }],
    },
  ]);
  assert.equal(
    parseStoredDraft(
      {
        form: { connectionBindings: [{ connectionId: "broken" }] },
        updatedAt: 42,
      },
      fallback,
    ),
    null,
  );
});

test("persists Agent drafts per workspace and clears them after use", () => {
  const values = new Map<string, string>();
  const originalWindow = globalThis.window;
  Object.defineProperty(globalThis, "window", {
    configurable: true,
    value: {
      localStorage: {
        getItem(key: string) {
          return values.get(key) ?? null;
        },
        removeItem(key: string) {
          values.delete(key);
        },
        setItem(key: string, value: string) {
          values.set(key, value);
        },
      },
    },
  });

  try {
    const form = {
      ...draftForm(),
      templateId: "audit-agent",
      name: "Audit Agent",
    };
    assert.equal(writeAgentDraft("J:\\Project\\OpenTopia", form), true);
    assert.deepEqual(
      readAgentDraft("J:\\Project\\OpenTopia", draftForm())?.form,
      form,
    );
    assert.equal(readAgentDraft("J:\\Project\\Other", form), null);

    clearAgentDraft("J:\\Project\\OpenTopia");
    assert.equal(readAgentDraft("J:\\Project\\OpenTopia", form), null);
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

function draftForm(): AgentDraftForm {
  return {
    templateId: "",
    name: "",
    owner: "enterprise-admin",
    description: "",
    instructions: "Complete the assigned task.",
    tools: "filesystem, shell",
    skills: "",
    plugins: "",
    legacyAllowAllMcpServers: false,
    mcpServers: "",
    connectionBindings: [],
    knowledgeProvider: "",
    knowledgeNamespaces: "",
    workspaceRoots: "J:\\Project\\OpenTopia",
    models: "",
    resourceGrants: "[]",
    stateSchema:
      '{"type":"object","properties":{},"additionalProperties":false}',
    outputSchema: '{"type":"object"}',
    delegates: "",
    riskClass: "medium",
  };
}
