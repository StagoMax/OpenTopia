import assert from "node:assert/strict";
import test from "node:test";

import { planRetries, summarizeRetries, validateJobs } from "../src/retry.js";

test("validates, canonicalizes, and sorts jobs", () => {
  assert.deepEqual(validateJobs([
    { id: "b", attempt: 0, maxAttempts: 2, baseDelayMs: 100, lastFailureAt: "2026-01-01T00:00:00Z" },
    { id: "a", attempt: 1, maxAttempts: 3, baseDelayMs: 50, lastFailureAt: "2026-01-01T00:00:01+00:00" },
  ]).map((job) => [job.id, job.lastFailureAt]), [
    ["a", "2026-01-01T00:00:01.000Z"],
    ["b", "2026-01-01T00:00:00.000Z"],
  ]);
});

test("plans ready, waiting, and exhausted retries", () => {
  const plan = planRetries([
    { id: "wait", attempt: 2, maxAttempts: 4, baseDelayMs: 1000, lastFailureAt: "2026-01-01T00:00:00Z" },
    { id: "ready", attempt: 0, maxAttempts: 3, baseDelayMs: 1000, lastFailureAt: "2025-12-31T23:59:00Z" },
    { id: "done", attempt: 3, maxAttempts: 3, baseDelayMs: 1000, lastFailureAt: "2026-01-01T00:00:00Z" },
  ], "2026-01-01T00:00:02Z");
  assert.deepEqual(plan.jobs.map((job) => [job.id, job.state, job.delayMs]), [
    ["ready", "ready", 1000],
    ["wait", "waiting", 4000],
    ["done", "exhausted", null],
  ]);
});

test("summarizes retry states", () => {
  const plan = planRetries([
    { id: "a", attempt: 1, maxAttempts: 4, baseDelayMs: 1000, lastFailureAt: "2026-01-01T00:00:00Z" },
    { id: "b", attempt: 4, maxAttempts: 4, baseDelayMs: 1000, lastFailureAt: "2026-01-01T00:00:00Z" },
  ], "2026-01-01T00:00:01Z");
  assert.deepEqual(summarizeRetries(plan), {
    jobs: 2,
    ready: 0,
    waiting: 1,
    exhausted: 1,
    nextWakeAt: "2026-01-01T00:00:02.000Z",
  });
});
