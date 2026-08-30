import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";

export const ATTRIBUTION_VERSION = 1;

const OWNER_LABELS = {
  engineering: "OpenTopia engineering",
  agent: "agent usage",
  task_environment: "task / benchmark environment",
  external: "external service",
  expected: "expected control flow",
  review: "manual review",
};

export function parseCsv(text) {
  const rows = [];
  let row = [];
  let field = "";
  let quoted = false;
  for (let index = 0; index < text.length; index += 1) {
    const character = text[index];
    if (quoted) {
      if (character === '"' && text[index + 1] === '"') {
        field += '"';
        index += 1;
      } else if (character === '"') {
        quoted = false;
      } else {
        field += character;
      }
      continue;
    }
    if (character === '"') {
      quoted = true;
    } else if (character === ",") {
      row.push(field);
      field = "";
    } else if (character === "\n") {
      row.push(field.replace(/\r$/, ""));
      rows.push(row);
      row = [];
      field = "";
    } else {
      field += character;
    }
  }
  if (quoted) throw new Error("Unterminated quoted CSV field");
  if (field.length > 0 || row.length > 0) {
    row.push(field.replace(/\r$/, ""));
    rows.push(row);
  }
  const [header, ...data] = rows.filter((candidate) => candidate.some(Boolean));
  if (!header) return [];
  return data.map((values) =>
    Object.fromEntries(header.map((key, index) => [key, values[index] ?? ""])),
  );
}

export function toolResultIsError(result) {
  const metadata = asRecord(result?.metadata);
  return Boolean(
    metadata &&
      (metadata.success === false ||
        metadata.isError === true ||
        hasOwn(metadata, "toolError") ||
        hasOwn(metadata, "errorRecord") ||
        hasOwn(metadata, "error")),
  );
}

export function classifyFailure(failure) {
  const message = failure.message.toLowerCase();
  const input = asRecord(failure.input) ?? {};

  if (
    message.includes("turn file mutation arrived after capture finalized") ||
    message.includes("failed to persist the turn file-mutation journal")
  ) {
    return attribution(
      "engineering",
      "file_mutation_journal",
      "high",
      "The tool operation was rolled back because OpenTopia finalized its mutation capture too early.",
    );
  }

  if (
    message.includes("preparesandbox") ||
    message.includes("sandbox setup failed") ||
    message.includes("acl ledger") ||
    message.includes("checktokenmembership") ||
    message.includes("privileged sandbox setup")
  ) {
    return attribution(
      "engineering",
      "sandbox_acl",
      "high",
      "OpenTopia sandbox preparation or ACL state blocked execution before the requested operation ran.",
    );
  }

  if (
    failure.code === "invalid_tool_arguments" &&
    failure.tool === "background_output"
  ) {
    const semantic = semanticBackgroundOutputInput(input);
    if (semantic.valid) {
      return attribution(
        "engineering",
        "tool_schema_compatibility",
        "high",
        `The requested ${semantic.action} operation had its required fields; irrelevant default fields were rejected by an exact-union schema.`,
      );
    }
    return attribution(
      "agent",
      "invalid_tool_arguments",
      "high",
      semantic.reason,
    );
  }

  if (
    failure.tool !== "shell" &&
    (failure.code === "execution_runtime_unavailable" ||
      message.includes("failed to run rg search") ||
      message.includes("execution failed during resolveruntime")) &&
    (message.includes("executable was not found") || message.includes("failed to run rg search"))
  ) {
    return attribution(
      "engineering",
      "bundled_tool_runtime_resolution",
      "high",
      "The built-in search tool could not resolve its own ripgrep runtime.",
    );
  }

  if (
    failure.message.includes("\\\\?\\") ||
    message.includes("extended-length path") ||
    message.includes("path normalization")
  ) {
    return attribution(
      "engineering",
      "path_normalization",
      "medium",
      "A platform-specific path representation escaped into a tool boundary that expected a normal user-visible path.",
    );
  }

  if (
    message === "cancelled" ||
    message.includes("operation cancelled") ||
    message.includes("operation canceled")
  ) {
    return attribution(
      "expected",
      "cancelled",
      "high",
      "Cancellation is an explicit control-flow outcome, not an engineering failure.",
    );
  }

  if (
    message.includes("timed out") ||
    message.includes("timeout") ||
    failure.code === "command_timeout"
  ) {
    return attribution(
      "task_environment",
      "command_timeout",
      "medium",
      "The requested task command exceeded its execution deadline.",
    );
  }

  if (
    message.includes("rate limit exceeded") ||
    message.includes("quota exceeded") ||
    message.includes("upstream request failed") ||
    message.includes("connection reset") ||
    message.includes("network request failed")
  ) {
    return attribution(
      "external",
      "external_service",
      "medium",
      "The recorded failure points to a provider or network dependency.",
    );
  }

  if (failure.code === "invalid_tool_arguments") {
    return attribution(
      "agent",
      "invalid_tool_arguments",
      "high",
      "The call omitted a required field or requested an unsupported action.",
    );
  }

  if (failure.tool === "apply_patch") {
    if (
      message.includes("made no changes") ||
      message.includes("cannot contain '..'") ||
      message.includes("invalid patch") ||
      message.includes("target already exists") ||
      message.includes("unsupported apply patch directive")
    ) {
      return attribution(
        "agent",
        "invalid_patch_request",
        "high",
        "The patch itself was a no-op or addressed a path outside the allowed patch scope.",
      );
    }
    if (
      message.includes("context") ||
      message.includes("old text") ||
      message.includes("does not match") ||
      message.includes("changed since")
    ) {
      return attribution(
        "agent",
        "patch_context_changed",
        "medium",
        "The patch no longer matched current file content; this is a stale/concurrent edit conflict, not a tool infrastructure failure.",
      );
    }
  }

  if (message.includes("approval required:")) {
    return attribution(
      "expected",
      "approval_boundary",
      "high",
      "The operation reached an explicit approval boundary; this is policy control flow, not a tool defect.",
    );
  }

  if (failure.tool === "update_plan") {
    return attribution(
      "agent",
      "invalid_tool_arguments",
      "high",
      "The plan update exceeded a declared input limit.",
    );
  }

  if (
    ["filesystem", "read_file", "read_files", "list_files", "search", "workspace_search"].includes(
      failure.tool,
    ) &&
    (message.includes("path does not exist") ||
      message.includes("no such file or directory") ||
      message.includes("cannot find the path") ||
      (failure.tool === "read_files" && message.includes("failed to read /")))
  ) {
    return attribution(
      "agent",
      "path_assumption",
      "medium",
      "The requested path was not present in the task workspace; no platform-path corruption was recorded.",
    );
  }

  if (
    failure.tool === "shell" ||
    failure.tool === "runtime_background_completion"
  ) {
    if (
      message.includes("modulenotfounderror") ||
      message.includes("no module named") ||
      message.includes("command not found") ||
      message.includes("executable was not found") ||
      message.includes("cannot import name")
    ) {
      return attribution(
        "task_environment",
        "task_dependency_unavailable",
        "medium",
        "A command selected by the agent depended on software absent or incompatible in the benchmark environment.",
      );
    }
    if (
      message.includes("syntaxerror") ||
      message.includes("parse error") ||
      message.includes("unexpected character after line continuation")
    ) {
      return attribution(
        "agent",
        "shell_command_construction",
        "high",
        "The generated shell or embedded-language command was syntactically invalid.",
      );
    }
    return attribution(
      "task_environment",
      "command_exit_nonzero",
      "medium",
      "The shell command or background process ran and returned a non-zero task-level result.",
    );
  }

  if (
    message.includes("path does not exist") ||
    message.includes("no such file or directory")
  ) {
    return attribution(
      "agent",
      "path_assumption",
      "low",
      "The recorded path was absent, but the event does not show platform-path corruption.",
    );
  }

  return attribution(
    "review",
    "unclassified",
    "low",
    "No high-confidence attribution rule matched this failure.",
  );
}

export function analyzeEvaluation({ terminalCsv, sweCsv }) {
  const runs = [
    ...loadRuns(terminalCsv, "terminal"),
    ...loadRuns(sweCsv, "swe"),
  ];
  const failures = [];

  for (const run of runs) {
    const events = JSON.parse(fs.readFileSync(run.eventPath, "utf8"));
    const calls = new Map();
    let started = 0;
    let finished = 0;
    for (const event of events) {
      const payload = asRecord(event.payload);
      if (payload?.type === "tool_call_started") {
        started += 1;
        const call = asRecord(payload.call);
        if (typeof call?.id === "string") calls.set(call.id, call);
        continue;
      }
      if (payload?.type !== "tool_call_finished") continue;
      finished += 1;
      const result = asRecord(payload.result);
      if (!toolResultIsError(result)) continue;
      const call = calls.get(result.callId) ?? {};
      const metadata = asRecord(result.metadata) ?? {};
      const errorRecord = asRecord(metadata.errorRecord) ?? {};
      const toolError = asRecord(metadata.toolError) ?? {};
      const message = firstText(
        errorRecord.message,
        metadata.error,
        typeof metadata.toolError === "string" ? metadata.toolError : null,
        toolError.message,
        toolError.error,
        result.output,
      );
      const failure = {
        benchmark: run.benchmark,
        snapshot: run.snapshot,
        task: run.task,
        turnId: typeof event.turnId === "string" ? event.turnId : "unknown",
        eventId: typeof event.id === "string" ? event.id : "unknown",
        callId: typeof result.callId === "string" ? result.callId : "unknown",
        tool: typeof call.name === "string" ? call.name : firstText(metadata.toolName) || "unknown",
        code: firstText(errorRecord.code) || "unspecified",
        message: message || "Tool result was marked as an error without a message.",
        input: asRecord(call.input) ?? {},
      };
      failures.push({ ...failure, attribution: classifyFailure(failure) });
    }
    run.started = started;
    run.finished = finished;
  }

  return {
    schemaVersion: ATTRIBUTION_VERSION,
    attributionOwners: OWNER_LABELS,
    runs: runs.map(({ eventPath: _eventPath, ...run }) => run),
    failures,
    summary: summarize(runs, failures),
  };
}

export function renderMarkdown(analysis, provenance) {
  const { terminal, swe, combined } = analysis.summary;
  const lines = [
    "# OpenTopia tool-call failure attribution: frozen Before / After",
    "",
    "## Scope and provenance",
    "",
    `- Before snapshot: \`${provenance.beforeVersion}\``,
    `- After snapshot: \`${provenance.afterVersion}\` (the frozen post-fix version used by the evaluation, not current HEAD)`,
    `- Workloads: Terminal-Bench (${terminal.before.runs} Before + ${terminal.after.runs} After runs) and SWE-bench Verified (${swe.before.runs} + ${swe.after.runs} runs)`,
    `- Coverage: ${combined.before.finished + combined.after.finished} finished tool calls; no model or benchmark rerun was performed`,
    `- Attribution ruleset: v${analysis.schemaVersion}; every error row is retained in the companion audit CSV`,
    "",
    "An engineering failure is counted only when OpenTopia blocked or rejected an otherwise executable operation (sandbox/ACL, mutation journal, internal runtime resolution, path normalization, or a semantically valid tool input rejected by schema). Shell/test failures, genuinely invalid arguments, missing task paths, patch conflicts, cancellation, and external failures are excluded.",
    "",
    "## Results",
    "",
    "| Workload | Snapshot | Finished calls | Raw failures | Raw failure rate | Engineering failures | Engineering failure rate | Engineering incidents | Review queue |",
    "|---|---|---:|---:|---:|---:|---:|---:|---:|",
    ...resultRows(terminal, "Terminal-Bench"),
    ...resultRows(swe, "SWE-bench Verified"),
    ...resultRows(combined, "Combined (diagnostic only)"),
    "",
    `Combined engineering-side failure rate changed from **${percent(combined.before.engineeringFailureRate)}** (${combined.before.engineeringFailures}/${combined.before.finished}) to **${percent(combined.after.engineeringFailureRate)}** (${combined.after.engineeringFailures}/${combined.after.finished}), a **${signedPoints(combined.after.engineeringFailureRate - combined.before.engineeringFailureRate)} percentage-point** change.`,
    "",
    "The combined figure is useful as a diagnostic inventory, not as a benchmark-quality claim: tool calls are clustered within tasks and the two workloads have different tool mixes.",
    "",
    "## Engineering causes",
    "",
    "| Cause | Before failures | After failures | Before incidents | After incidents |",
    "|---|---:|---:|---:|---:|",
    ...engineeringCauseRows(combined.before, combined.after),
    "",
    "## Interpretation",
    "",
    ...interpretationLines(combined),
    "",
    "## What this does and does not validate",
    "",
    "- The frozen data validates the exact binaries named above. It does not validate fixes implemented after the After snapshot.",
    ...(provenance.currentValidation
      ? [
          `- Current targeted validation: ${provenance.currentValidation}. These tests establish regression coverage, but they do not retroactively change the frozen benchmark rate.`,
        ]
      : [
          "- Current mutation-journal, Windows ACL/path, and schema changes should be cited as regression-covered only after their targeted tests or a small replay pass.",
        ]),
    "- A full public benchmark rerun is unnecessary unless a new end-to-end benchmark number for current HEAD is required.",
    "- The audit CSV omits absolute source paths and full tool inputs. Event/call IDs allow a reviewer with access to the original artifacts to trace any classification.",
    "",
  ];
  return `${lines.join("\n")}\n`;
}

export function renderAuditCsv(analysis) {
  const header = [
    "benchmark",
    "snapshot",
    "task",
    "turn_id",
    "event_id",
    "call_id",
    "tool",
    "code",
    "owner",
    "cause",
    "confidence",
    "message_fingerprint",
    "message_excerpt",
    "rationale",
  ];
  const rows = analysis.failures.map((failure) => [
    failure.benchmark,
    failure.snapshot,
    failure.task,
    failure.turnId,
    failure.eventId,
    failure.callId,
    failure.tool,
    failure.code,
    failure.attribution.owner,
    failure.attribution.cause,
    failure.attribution.confidence,
    fingerprint(failure.message),
    excerpt(failure.message),
    failure.attribution.rationale,
  ]);
  return [header, ...rows].map((row) => row.map(csvField).join(",")).join("\n") + "\n";
}

export function serializableAnalysis(analysis, provenance) {
  return {
    schemaVersion: analysis.schemaVersion,
    generatedAt: new Date().toISOString(),
    provenance,
    methodology: {
      denominator: "finished tool calls",
      engineeringDefinition:
        "OpenTopia blocked or rejected an otherwise executable operation",
      incidentKey: "benchmark + snapshot + task + turn + cause",
      rawInputsRetained: false,
    },
    attributionOwners: analysis.attributionOwners,
    summary: analysis.summary,
    audit: {
      errorRows: analysis.failures.length,
      reviewRows: analysis.failures.filter(
        (failure) => failure.attribution.owner === "review",
      ).length,
    },
  };
}

function loadRuns(csvFile, benchmark) {
  return parseCsv(fs.readFileSync(csvFile, "utf8")).map((row) => {
    if (!row.source_path) throw new Error(`Missing source_path in ${csvFile}`);
    if (row.snapshot !== "before" && row.snapshot !== "after") {
      throw new Error(`Unexpected snapshot '${row.snapshot}' in ${csvFile}`);
    }
    const task = benchmark === "terminal" ? row.task : row.instance_id;
    return {
      benchmark,
      snapshot: row.snapshot,
      task,
      eventPath: path.join(
        path.dirname(row.source_path),
        benchmark === "terminal" ? "agent" : "agent-logs",
        "opentopia-events.json",
      ),
      started: 0,
      finished: 0,
    };
  });
}

function summarize(runs, failures) {
  const result = {};
  for (const benchmark of ["terminal", "swe", "combined"]) {
    result[benchmark] = {};
    for (const snapshot of ["before", "after"]) {
      const selectedRuns = runs.filter(
        (run) =>
          run.snapshot === snapshot &&
          (benchmark === "combined" || run.benchmark === benchmark),
      );
      const selectedFailures = failures.filter(
        (failure) =>
          failure.snapshot === snapshot &&
          (benchmark === "combined" || failure.benchmark === benchmark),
      );
      const engineering = selectedFailures.filter(
        (failure) => failure.attribution.owner === "engineering",
      );
      const finished = sum(selectedRuns, "finished");
      const rawFailures = selectedFailures.length;
      const incidentSet = new Set(
        engineering.map((failure) =>
          [
            failure.benchmark,
            failure.snapshot,
            failure.task,
            failure.turnId,
            failure.attribution.cause,
          ].join("\0"),
        ),
      );
      result[benchmark][snapshot] = {
        runs: selectedRuns.length,
        started: sum(selectedRuns, "started"),
        finished,
        lifecycleCompletionRate: ratio(sum(selectedRuns, "finished"), sum(selectedRuns, "started")),
        rawFailures,
        rawFailureRate: ratio(rawFailures, finished),
        engineeringFailures: engineering.length,
        engineeringFailureRate: ratio(engineering.length, finished),
        engineeringIncidents: incidentSet.size,
        reviewFailures: selectedFailures.filter(
          (failure) => failure.attribution.owner === "review",
        ).length,
        ownerCounts: countBy(selectedFailures, (failure) => failure.attribution.owner),
        causeCounts: countBy(selectedFailures, (failure) => failure.attribution.cause),
        engineeringCauseIncidents: countEngineeringIncidents(engineering),
        engineeringRateWilson95: wilson(engineering.length, finished),
      };
    }
  }
  return result;
}

function countEngineeringIncidents(failures) {
  const byCause = new Map();
  for (const failure of failures) {
    const incidents = byCause.get(failure.attribution.cause) ?? new Set();
    incidents.add(
      [failure.benchmark, failure.task, failure.turnId, failure.attribution.cause].join("\0"),
    );
    byCause.set(failure.attribution.cause, incidents);
  }
  return Object.fromEntries([...byCause].map(([cause, incidents]) => [cause, incidents.size]));
}

function semanticBackgroundOutputInput(input) {
  const action = input.action;
  if (action === "list") return { valid: true, action };
  if (action === "read" || action === "stop") {
    return typeof input.jobId === "string" && input.jobId.length > 0
      ? { valid: true, action }
      : { valid: false, action, reason: `background_output ${action} genuinely omitted jobId.` };
  }
  if (action === "write") {
    if (typeof input.jobId !== "string" || input.jobId.length === 0) {
      return { valid: false, action, reason: "background_output write genuinely omitted jobId." };
    }
    if (typeof input.data !== "string") {
      return { valid: false, action, reason: "background_output write genuinely omitted string data." };
    }
    return { valid: true, action };
  }
  return { valid: false, action, reason: "background_output requested an unsupported action." };
}

function attribution(owner, cause, confidence, rationale) {
  return { owner, cause, confidence, rationale };
}

function interpretationLines(combined) {
  const before = combined.before;
  const after = combined.after;
  const journal = after.causeCounts.file_mutation_journal ?? 0;
  const schema = after.causeCounts.tool_schema_compatibility ?? 0;
  const runtimeBefore = before.causeCounts.bundled_tool_runtime_resolution ?? 0;
  const lines = [];
  if (journal > 0) {
    lines.push(
      `- After contains **${journal}** mutation-journal failures. They are cascading tool results rather than ${journal} independent root bugs; the incident count above deduplicates by task, turn, and cause.`,
    );
  }
  if (schema > 0) {
    lines.push(
      `- After contains **${schema}** semantically valid \`background_output\` calls rejected because irrelevant default fields were present. Genuinely incomplete calls remain attributed to the agent.`,
    );
  }
  if (runtimeBefore > 0) {
    lines.push(
      `- Before contains **${runtimeBefore}** failures where built-in tools could not resolve their own \`rg\` or \`git\` runtime; shell commands that independently chose unavailable binaries are not counted as this engineering cause.`,
    );
  }
  if (after.engineeringFailureRate > before.engineeringFailureRate) {
    lines.push(
      "- On these frozen snapshots, engineering-side tool failure did **not** improve. The historical After snapshot therefore cannot support a résumé claim that the final fixes reduced the engineering failure rate.",
    );
  } else {
    lines.push(
      "- On these frozen snapshots, engineering-side tool failure improved under the stated attribution rules.",
    );
  }
  return lines;
}

function resultRows(summary, label) {
  return ["before", "after"].map((snapshot) => {
    const value = summary[snapshot];
    return `| ${label} | ${snapshot === "before" ? "Before" : "After"} | ${value.finished} | ${value.rawFailures} | ${percent(value.rawFailureRate)} | ${value.engineeringFailures} | ${percent(value.engineeringFailureRate)} | ${value.engineeringIncidents} | ${value.reviewFailures} |`;
  });
}

function engineeringCauseRows(before, after) {
  const causes = new Set([
    ...Object.keys(before.engineeringCauseIncidents),
    ...Object.keys(after.engineeringCauseIncidents),
  ]);
  return [...causes]
    .sort()
    .map(
      (cause) =>
        `| \`${cause}\` | ${before.causeCounts[cause] ?? 0} | ${after.causeCounts[cause] ?? 0} | ${before.engineeringCauseIncidents[cause] ?? 0} | ${after.engineeringCauseIncidents[cause] ?? 0} |`,
    );
}

function wilson(successes, trials) {
  if (trials === 0) return null;
  const z = 1.959963984540054;
  const p = successes / trials;
  const denominator = 1 + (z * z) / trials;
  const center = (p + (z * z) / (2 * trials)) / denominator;
  const margin =
    (z / denominator) *
    Math.sqrt((p * (1 - p)) / trials + (z * z) / (4 * trials * trials));
  return [Math.max(0, center - margin), Math.min(1, center + margin)];
}

function countBy(items, keyOf) {
  const counts = {};
  for (const item of items) {
    const key = keyOf(item);
    counts[key] = (counts[key] ?? 0) + 1;
  }
  return Object.fromEntries(Object.entries(counts).sort(([left], [right]) => left.localeCompare(right)));
}

function sum(items, key) {
  return items.reduce((total, item) => total + item[key], 0);
}

function ratio(numerator, denominator) {
  return denominator === 0 ? null : numerator / denominator;
}

function percent(value) {
  return value === null ? "n/a" : `${(value * 100).toFixed(2)}%`;
}

function signedPoints(value) {
  const points = value * 100;
  return `${points >= 0 ? "+" : ""}${points.toFixed(2)}`;
}

function fingerprint(message) {
  return crypto.createHash("sha256").update(message).digest("hex").slice(0, 16);
}

function excerpt(message) {
  return message.replace(/\s+/g, " ").trim().slice(0, 300);
}

function csvField(value) {
  const text = String(value ?? "");
  return /[",\r\n]/.test(text) ? `"${text.replaceAll('"', '""')}"` : text;
}

function asRecord(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value
    : null;
}

function hasOwn(record, key) {
  return Object.prototype.hasOwnProperty.call(record, key);
}

function firstText(...values) {
  for (const value of values) {
    if (typeof value === "string" && value.trim().length > 0) return value.trim();
  }
  return "";
}
