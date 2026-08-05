import { readFile, readdir, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import { ensureDirectory } from "./utils.mjs";

const MAX_SUMMARIES = 500;
const MAX_SUMMARY_BYTES = 2 * 1024 * 1024;

function text(value) {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function runTimestamp(run) {
  const value = run.completedAt ?? run.startedAt;
  const timestamp = Date.parse(value ?? "");
  return Number.isFinite(timestamp) ? timestamp : 0;
}

function taskAttempts(summary) {
  if (Array.isArray(summary.results) && summary.results.length > 0) {
    return summary.results.map((result) => ({
      taskId: text(result.taskId) ?? "task",
      status: text(result.status) ?? "unknown",
      failureCategory: text(result.failureCategory),
      error: text(result.error)
    }));
  }
  const tasks = Array.isArray(summary.tasks) ? summary.tasks : [];
  const attempts = [];
  for (const task of tasks) {
    const taskId = text(task.taskId) ?? text(task.task) ?? text(task.id) ?? "task";
    const runs = Array.isArray(task.runs) && task.runs.length > 0
      ? task.runs
      : Array.isArray(task.statuses) && task.statuses.length > 0
        ? task.statuses.map((status) => ({ status }))
        : [task];
    for (const run of runs) {
      attempts.push({
        taskId,
        status: text(run.status) ?? "unknown",
        failureCategory: text(run.failureCategory),
        error: text(run.error)
      });
    }
  }
  return attempts;
}

function normalizeSummary(directory, summaryPath, summary, hasReport) {
  const attempts = taskAttempts(summary);
  const passed = attempts.filter((attempt) => attempt.status === "passed").length;
  const total = attempts.length;
  const aggregatePassRate = summary.aggregate?.passRate;
  const inferredStatus = total === 0 ? "unknown" : passed === total ? "passed" : "failed";
  return {
    runId:
      text(summary.runId) ??
      text(summary.suiteId) ??
      text(summary.suite?.id) ??
      path.basename(directory),
    title:
      text(summary.benchmark) ??
      text(summary.title) ??
      text(summary.suite?.title) ??
      text(summary.suiteId) ??
      path.basename(directory),
    status: text(summary.status) ?? inferredStatus,
    model: text(summary.model) ?? text(summary.provider?.model) ?? text(summary.provider?.expectedModel),
    startedAt: text(summary.startedAt),
    completedAt: text(summary.completedAt),
    passed,
    total,
    passRate:
      typeof aggregatePassRate === "number"
        ? aggregatePassRate
        : total > 0
          ? passed / total
          : null,
    attempts,
    directory,
    summaryPath,
    reportPath: hasReport ? path.join(directory, "report.md") : null
  };
}

async function isFile(filePath) {
  try {
    return (await stat(filePath)).isFile();
  } catch (error) {
    if (error.code === "ENOENT") return false;
    throw error;
  }
}

export async function scanEvaluationRuns(rootDirectory) {
  const root = path.resolve(rootDirectory);
  let entries;
  try {
    entries = await readdir(root, { withFileTypes: true });
  } catch (error) {
    if (error.code === "ENOENT") return { rootDirectory: root, runs: [], warnings: [] };
    throw error;
  }

  const directories = entries
    .filter((entry) => entry.isDirectory())
    .slice(0, MAX_SUMMARIES);
  const runs = [];
  const warnings = [];
  for (const entry of directories) {
    const directory = path.join(root, entry.name);
    const summaryPath = path.join(directory, "summary.json");
    if (!(await isFile(summaryPath))) continue;
    try {
      const metadata = await stat(summaryPath);
      if (metadata.size > MAX_SUMMARY_BYTES) {
        throw new Error(`summary exceeds ${MAX_SUMMARY_BYTES} bytes`);
      }
      const summary = JSON.parse(await readFile(summaryPath, "utf8"));
      runs.push(
        normalizeSummary(
          directory,
          summaryPath,
          summary,
          await isFile(path.join(directory, "report.md"))
        )
      );
    } catch (error) {
      warnings.push({ path: summaryPath, error: error.message });
    }
  }
  runs.sort((left, right) => runTimestamp(right) - runTimestamp(left) || right.runId.localeCompare(left.runId));
  return { rootDirectory: root, runs, warnings };
}

function markdownText(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll("|", "\\|")
    .replaceAll("\r", " ")
    .replaceAll("\n", " ");
}

function relativeLink(outputPath, targetPath) {
  return path
    .relative(path.dirname(outputPath), targetPath)
    .split(path.sep)
    .map(encodeURIComponent)
    .join("/");
}

function percentage(value) {
  return value === null ? "n/a" : `${(value * 100).toFixed(1)}%`;
}

export function renderEvaluationCatalog(catalog, outputPath) {
  const lines = [
    "# Evaluation Catalog",
    "",
    `Generated: ${new Date().toISOString()}`,
    "",
    `Runs: ${catalog.runs.length}`,
    "",
    "| Run | Suite | Status | Model | Passed | Completed | Artifacts |",
    "|---|---|---|---|---:|---|---|"
  ];
  for (const run of catalog.runs) {
    const summaryLink = relativeLink(outputPath, run.summaryPath);
    const artifactLink = run.reportPath
      ? `[report](<${relativeLink(outputPath, run.reportPath)}>) · [summary](<${summaryLink}>)`
      : `[summary](<${summaryLink}>)`;
    const passed = run.total > 0 ? `${run.passed}/${run.total} (${percentage(run.passRate)})` : percentage(run.passRate);
    lines.push(
      `| ${markdownText(run.runId)} | ${markdownText(run.title)} | ${markdownText(run.status)} | ${markdownText(run.model ?? "n/a")} | ${passed} | ${markdownText(run.completedAt ?? run.startedAt ?? "n/a")} | ${artifactLink} |`
    );
  }

  const failedAttempts = catalog.runs.flatMap((run) =>
    run.attempts
      .filter((attempt) => attempt.status !== "passed")
      .map((attempt) => ({ runId: run.runId, ...attempt }))
  );
  lines.push("", "## Failed Attempts", "");
  if (failedAttempts.length === 0) {
    lines.push("None.");
  } else {
    lines.push("| Run | Task | Status | Category | Error |", "|---|---|---|---|---|");
    for (const attempt of failedAttempts.slice(0, 200)) {
      lines.push(
        `| ${markdownText(attempt.runId)} | ${markdownText(attempt.taskId)} | ${markdownText(attempt.status)} | ${markdownText(attempt.failureCategory ?? "n/a")} | ${markdownText(attempt.error ?? "")} |`
      );
    }
  }

  if (catalog.warnings.length > 0) {
    lines.push("", "## Skipped Summaries", "");
    for (const warning of catalog.warnings) {
      lines.push(`- \`${markdownText(warning.path)}\`: ${markdownText(warning.error)}`);
    }
  }
  lines.push("");
  return `${lines.join("\n")}\n`;
}

export async function writeEvaluationCatalog(rootDirectory, outputPath) {
  const resolvedOutput = path.resolve(outputPath);
  const catalog = await scanEvaluationRuns(rootDirectory);
  await ensureDirectory(path.dirname(resolvedOutput));
  await writeFile(resolvedOutput, renderEvaluationCatalog(catalog, resolvedOutput), "utf8");
  return { ...catalog, outputPath: resolvedOutput };
}
