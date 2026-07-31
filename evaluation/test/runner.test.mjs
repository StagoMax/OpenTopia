import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { runSuite, validateDefinitions } from "../src/runner.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const example = path.resolve(here, "../examples/smoke");
const recoveryExample = path.resolve(here, "../examples/recovery-smoke");

test("validates the example without loading product code", async () => {
  const definitions = await validateDefinitions(
    path.join(example, "suite.json"),
    path.join(example, "target.json")
  );
  assert.equal(definitions.tasks.length, 1);
  assert.equal(definitions.target.id, "black-box-mock");
});

test("runs a black-box trial and produces separated scores and cache metrics", async () => {
  const output = await mkdtemp(path.join(os.tmpdir(), "agent-eval-test-"));
  try {
    const result = await runSuite({
      suitePath: path.join(example, "suite.json"),
      targetPath: path.join(example, "target.json"),
      outputDirectory: output,
      repetitions: 1
    });
    assert.equal(result.summary.status, "passed");
    assert.equal(result.summary.results[0].status, "passed");
    assert.deepEqual(result.summary.results[0].scores, {
      outcome: true,
      trajectory: true,
      safety: true,
      efficiency: true
    });
    assert.equal(result.summary.results[0].metrics.usage.cachedInputRatio, 0.7);
    assert.equal(result.summary.results[0].metrics.usage.cacheTelemetryCoverage, 1);
    assert.equal(result.summary.results[0].metrics.multiAgent.orphaned, 0);
    assert.equal(result.summary.results[0].metrics.browser.actionValidity, 1);
    const report = await readFile(result.reports.markdownPath, "utf8");
    assert.match(report, /Strict success: 1\/1/);
    assert.doesNotMatch(report, /EVAL_CANARY_DO_NOT_LEAK_8421/);
  } finally {
    await rm(output, { recursive: true, force: true });
  }
});

test("runs staged recovery with hidden graders before and after restart", async () => {
  const output = await mkdtemp(path.join(os.tmpdir(), "agent-eval-recovery-test-"));
  try {
    const result = await runSuite({
      suitePath: path.join(recoveryExample, "suite.json"),
      targetPath: path.join(recoveryExample, "target.json"),
      outputDirectory: output,
      repetitions: 1
    });
    const trial = result.summary.results[0];
    assert.equal(result.summary.status, "passed");
    assert.equal(trial.status, "passed");
    assert.equal(trial.process.stages.length, 2);
    assert.deepEqual(trial.process.stages.map((stage) => stage.graderPassed), [true, true]);
    assert.equal(trial.metrics.longHorizon.successfulRecoveries, 1);
    assert.ok(trial.checks.some((check) => check.id === "phase.prepare.phase-one-progress" && check.passed));
    assert.ok(trial.checks.some((check) => check.id === "phase.resume.final-progress" && check.passed));
    const events = await readFile(trial.artifacts.events, "utf8");
    assert.match(events, /application\.recovery\.restart\.completed/);
  } finally {
    await rm(output, { recursive: true, force: true });
  }
});
