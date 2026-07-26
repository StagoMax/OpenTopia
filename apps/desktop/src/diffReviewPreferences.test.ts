import assert from "node:assert/strict";
import test from "node:test";

import type * as DiffReviewPreferencesModule from "./diffReviewPreferences";

const preferences: typeof DiffReviewPreferencesModule = await import(
  "./diffReviewPreferences" + ".ts"
);

const { defaultDiffReviewPreferences, normalizeDiffReviewPreferences } =
  preferences;

test("keeps the defaults for an empty or unusable payload", () => {
  assert.deepEqual(
    normalizeDiffReviewPreferences(undefined),
    defaultDiffReviewPreferences,
  );
  assert.deepEqual(
    normalizeDiffReviewPreferences({}),
    defaultDiffReviewPreferences,
  );
});

test("keeps valid fields and drops the rest", () => {
  assert.deepEqual(
    normalizeDiffReviewPreferences({
      view: "unified",
      wrapLines: true,
      hideWhitespace: "yes",
      view2: "split",
    }),
    {
      ...defaultDiffReviewPreferences,
      view: "unified",
      wrapLines: true,
    },
  );
});

test("rejects an unknown view", () => {
  assert.equal(
    normalizeDiffReviewPreferences({ view: "inline" }).view,
    defaultDiffReviewPreferences.view,
  );
});
