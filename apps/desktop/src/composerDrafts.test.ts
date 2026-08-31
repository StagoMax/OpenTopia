import assert from "node:assert/strict";
import test from "node:test";

import type * as ComposerDraftsModule from "./composerDrafts";

const {
  getComposerDraftsSnapshot,
  newTaskComposerDraftKey,
  parseComposerDrafts,
  threadComposerDraftKey,
  updateComposerDraft,
} = (await import("./composerDrafts" + ".ts")) as typeof ComposerDraftsModule;

test("uses separate durable keys for conversations and project drafts", () => {
  assert.equal(threadComposerDraftKey("thread-a"), "thread:thread-a");
  assert.equal(
    newTaskComposerDraftKey("code", "project-a"),
    "new:code:project-a",
  );
  assert.equal(newTaskComposerDraftKey("work", null), "new:work:unassigned");
});

test("restores text, context sources, and skills while rejecting malformed data", () => {
  assert.deepEqual(
    parseComposerDrafts({
      "thread:a": {
        text: "你好",
        contextSources: [
          {
            path: "J:\\Project\\a.ts",
            name: "a.ts",
            extension: ".ts",
            kind: "text",
            bytes: 12,
          },
          { path: "broken" },
        ],
        selectedSkillIds: ["review", "review", 1],
        updatedAt: 10,
      },
      "thread:empty": {
        text: "",
        contextSources: [],
        selectedSkillIds: [],
        updatedAt: 20,
      },
    }),
    {
      "thread:a": {
        text: "你好",
        contentParts: [{ type: "text", text: "你好" }],
        contextSources: [
          {
            path: "J:\\Project\\a.ts",
            name: "a.ts",
            extension: ".ts",
            kind: "text",
            bytes: 12,
          },
        ],
        selectedSkillIds: ["review"],
        updatedAt: 10,
      },
    },
  );
});

test("round-trips attachment references and upgrades matching legacy markers", () => {
  const source = {
    path: "J:\\Project\\需求说明.pdf",
    name: "需求说明.pdf",
    extension: ".pdf",
    kind: "document" as const,
    bytes: 42,
  };
  const contentParts = [
    { type: "text" as const, text: "请查看" },
    {
      type: "attachment_ref" as const,
      path: source.path,
      name: source.name,
    },
    { type: "text" as const, text: "中的结论" },
  ];

  const parsed = parseComposerDrafts({
    "thread:structured": {
      text: "请查看[需求说明.pdf]中的结论",
      contentParts,
      contextSources: [source],
      selectedSkillIds: [],
      updatedAt: 20,
    },
    "thread:legacy": {
      text: "请查看[需求说明.pdf]中的结论",
      contextSources: [source],
      selectedSkillIds: [],
      updatedAt: 10,
    },
    "thread:ambiguous": {
      text: "请查看[需求说明.pdf]中的结论",
      contextSources: [
        source,
        { ...source, path: "J:\\Project\\副本\\需求说明.pdf" },
      ],
      selectedSkillIds: [],
      updatedAt: 5,
    },
  });

  assert.deepEqual(parsed["thread:structured"]?.contentParts, contentParts);
  assert.deepEqual(parsed["thread:legacy"]?.contentParts, contentParts);
  assert.deepEqual(parsed["thread:ambiguous"]?.contentParts, [
    { type: "text", text: "请查看[需求说明.pdf]中的结论" },
  ]);
});

test("updates and clears one conversation without affecting another", () => {
  updateComposerDraft("thread:first", (draft) => ({
    ...draft,
    contentParts: [{ type: "text", text: "first draft" }],
  }));
  updateComposerDraft("thread:second", (draft) => ({
    ...draft,
    contentParts: [{ type: "text", text: "second draft" }],
  }));
  assert.equal(
    getComposerDraftsSnapshot()["thread:first"]?.text,
    "first draft",
  );
  assert.equal(
    getComposerDraftsSnapshot()["thread:second"]?.text,
    "second draft",
  );

  updateComposerDraft("thread:first", () => ({
    text: "",
    contentParts: [],
    contextSources: [],
    selectedSkillIds: [],
    updatedAt: 0,
  }));
  assert.equal(getComposerDraftsSnapshot()["thread:first"], undefined);
  assert.equal(
    getComposerDraftsSnapshot()["thread:second"]?.text,
    "second draft",
  );
});
