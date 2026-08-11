import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import {
  buildRegressionReport,
  validateRegressionRegistry,
  writeRegressionReport,
} from "../src/regressions.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const registryPath = path.resolve(here, "../regressions/registry.json");

test("validates the durable regression registry and all executable links", async () => {
  const validated = await validateRegressionRegistry(registryPath);
  assert.equal(validated.registry.id, "opentopia-regression-registry");
  assert.equal(validated.registry.cases.length, 11);
  assert.ok(validated.registry.cases.some((entry) => entry.kind === "incident"));
  assert.ok(validated.registry.cases.some((entry) => entry.area === "cross_tool"));
  assert.ok(validated.registry.cases.some((entry) => entry.area === "recovery"));
});

test("maps supplied evaluation summaries back to historical cases", async () => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "opentopia-regressions-"));
  const summaryPath = path.join(directory, "summary.json");
  try {
    await writeFile(summaryPath, JSON.stringify({
      runId: "candidate-run",
      suite: { id: "opentopia-architecture-calibration-v1" },
      results: [
        { trialId: "CAL-ROUTER-001_1", taskId: "CAL-ROUTER-001", status: "passed" },
        { trialId: "CAL-ASYNC-001_1", taskId: "CAL-ASYNC-001", status: "task_failed" },
      ],
    }), "utf8");

    const report = await buildRegressionReport({ registryPath, summaryPaths: [summaryPath] });
    const router = report.cases.find((entry) => entry.id === "REG-20260810-ROUTER-LITERAL");
    const asyncAbort = report.cases.find((entry) => entry.id === "REG-20260810-ASYNC-ABORT-REASON");
    assert.equal(router.passRate, 1);
    assert.equal(asyncAbort.passRate, 0);
    assert.equal(report.aggregate.evaluatedCases, 3);
    assert.equal(report.aggregate.validAttempts, 3);
    assert.equal(report.aggregate.passedAttempts, 1);

    const outputPath = path.join(directory, "regressions.md");
    const written = await writeRegressionReport({
      registryPath,
      summaryPaths: [summaryPath],
      outputPath,
    });
    const markdown = await readFile(written.markdownPath, "utf8");
    assert.match(markdown, /REG-20260810-ROUTER-LITERAL/);
    assert.match(markdown, /1\/1 \(100\.0%\)/);
    assert.match(markdown, /Open work/);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("rejects a registry whose executable source-test anchor disappeared", async () => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "opentopia-regression-invalid-"));
  const sourcePath = path.join(directory, "fixture.test.mjs");
  const invalidRegistryPath = path.join(directory, "registry.json");
  try {
    await writeFile(sourcePath, "test('present anchor', () => {});\n", "utf8");
    await writeFile(invalidRegistryPath, JSON.stringify({
      schemaVersion: 1,
      id: "invalid-registry",
      title: "Invalid registry",
      cases: [{
        id: "RISK-MISSING-ANCHOR",
        kind: "risk",
        title: "Missing anchor",
        state: "monitoring",
        severity: "low",
        area: "grader",
        gate: "smoke",
        firstObservedAt: "2026-08-11T00:00:00.000Z",
        origin: { kind: "risk-analysis", reference: "validator test" },
        observedBehavior: "A renamed test can leave a registry link stale.",
        rootCause: { status: "not_applicable", summary: "Preventive validator coverage." },
        expectedBehavior: "Validation fails when the anchor no longer exists.",
        tags: ["validator"],
        coverage: [{
          kind: "source-test",
          purpose: "regression",
          file: "fixture.test.mjs",
          anchor: "missing anchor",
          command: "node --test fixture.test.mjs"
        }]
      }]
    }), "utf8");

    await assert.rejects(
      validateRegressionRegistry(invalidRegistryPath),
      /source test anchor not found/,
    );
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
