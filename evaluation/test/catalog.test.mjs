import assert from "node:assert/strict";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { scanEvaluationRuns, writeEvaluationCatalog } from "../src/catalog.mjs";

test("catalogs harness and script summaries outside the product", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "agent-eval-catalog-"));
  try {
    const harnessDirectory = path.join(root, "harness-run");
    const scriptDirectory = path.join(root, "script-run");
    await mkdir(harnessDirectory);
    await mkdir(scriptDirectory);
    await writeFile(
      path.join(harnessDirectory, "summary.json"),
      JSON.stringify({
        runId: "harness-run",
        suite: { id: "tools", title: "Tool Suite" },
        status: "passed",
        completedAt: "2026-08-05T10:00:00.000Z",
        aggregate: { passRate: 1 },
        tasks: [{ taskId: "read", statuses: ["passed", "passed"] }]
      }),
      "utf8"
    );
    await writeFile(path.join(harnessDirectory, "report.md"), "# Report\n", "utf8");
    await writeFile(
      path.join(scriptDirectory, "summary.json"),
      JSON.stringify({
        runId: "script-run",
        benchmark: "Browser Suite",
        model: "test-model",
        completedAt: "2026-08-05T11:00:00.000Z",
        tasks: [{ task: "navigate", status: "failed", error: "timeout" }]
      }),
      "utf8"
    );

    const catalog = await scanEvaluationRuns(root);
    assert.deepEqual(catalog.runs.map((run) => run.runId), ["script-run", "harness-run"]);
    assert.equal(catalog.runs[0].status, "failed");
    assert.equal(catalog.runs[0].attempts[0].error, "timeout");
    assert.equal(catalog.runs[1].passRate, 1);
    assert.equal(catalog.runs[1].total, 2);

    const outputPath = path.join(root, "index.md");
    await writeEvaluationCatalog(root, outputPath);
    const markdown = await readFile(outputPath, "utf8");
    assert.match(markdown, /Browser Suite/);
    assert.match(markdown, /Failed Attempts/);
    assert.match(markdown, /\[report\]/);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("skips malformed summaries and reports the warning", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "agent-eval-catalog-invalid-"));
  try {
    const invalidDirectory = path.join(root, "invalid");
    await mkdir(invalidDirectory);
    await writeFile(path.join(invalidDirectory, "summary.json"), "{", "utf8");
    const catalog = await scanEvaluationRuns(root);
    assert.equal(catalog.runs.length, 0);
    assert.equal(catalog.warnings.length, 1);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
