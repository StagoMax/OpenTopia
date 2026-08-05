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

test("updates and clears one conversation without affecting another", () => {
  updateComposerDraft("thread:first", (draft) => ({
    ...draft,
    text: "first draft",
  }));
  updateComposerDraft("thread:second", (draft) => ({
    ...draft,
    text: "second draft",
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
