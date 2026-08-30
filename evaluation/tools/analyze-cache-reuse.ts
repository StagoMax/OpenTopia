import fs from "node:fs";
import path from "node:path";

import { aggregateUsageEvents } from "../../apps/desktop/src/usageLogs.ts";

type CsvRow = Record<string, string>;
type RunKind = "terminal" | "swe";

type CacheCall = {
  round: number;
  input: number;
  cached: number;
};

type RunSummary = {
  id: string;
  snapshot: string;
  calls: CacheCall[];
  eligibleTransitions: number;
  broken: number;
  degraded: number;
  reasons: Record<string, number>;
  breakpointKinds: Record<string, number>;
  breakpointChanges: Record<string, number>;
  tailInput: number;
  tailCached: number;
  tailMedian: number | null;
  tailP10: number | null;
  tailP90: number | null;
};

function requiredArgument(name: string): string {
  const index = process.argv.indexOf(name);
  const value = index >= 0 ? process.argv[index + 1] : undefined;
  if (!value || value.startsWith("--")) {
    throw new Error(`Missing required argument: ${name}`);
  }
  return value;
}

function readCsv(file: string): CsvRow[] {
  const [header, ...lines] = fs
    .readFileSync(file, "utf8")
    .trim()
    .split(/\r?\n/);
  const keys = header.split(",");
  return lines.map((line) => {
    const values = line.split(",");
    return Object.fromEntries(
      keys.map((key, index) => [key, values[index] ?? ""]),
    );
  });
}

function quantile(values: number[], q: number): number | null {
  const items = [...values].sort((left, right) => left - right);
  if (items.length === 0) return null;
  const position = (items.length - 1) * q;
  const low = Math.floor(position);
  const high = Math.ceil(position);
  return items[low]! + (items[high]! - items[low]!) * (position - low);
}

function eventLogPath(sourcePath: string, kind: RunKind): string {
  return path.join(
    path.dirname(sourcePath),
    kind === "terminal" ? "agent" : "agent-logs",
    "opentopia-events.json",
  );
}

function analyzeRuns(
  csvFile: string,
  kind: RunKind,
  idField: string,
): RunSummary[] {
  return readCsv(csvFile).map((row) => {
    const sourcePath = row.source_path;
    if (!sourcePath) throw new Error(`Missing source_path in ${csvFile}`);
    const eventsPath = eventLogPath(sourcePath, kind);
    const dashboard = aggregateUsageEvents(
      JSON.parse(fs.readFileSync(eventsPath, "utf8")),
    );
    const calls = dashboard.calls.filter(
      (call) => call.cacheReadTokensReported && call.inputTokens > 0,
    );
    const eligible = calls.filter(
      (call) =>
        call.cacheReuse.previousCachedInputTokens !== null &&
        call.cacheReuse.previousCachedInputTokens > 0,
    );
    const damaged = eligible.filter(
      (call) =>
        call.cacheReuse.state === "broken" ||
        call.cacheReuse.state === "degraded",
    );
    const reasons: Record<string, number> = {};
    const breakpointKinds: Record<string, number> = {};
    const breakpointChanges: Record<string, number> = {};
    for (const call of damaged) {
      const reason = call.cacheReuse.reason;
      reasons[reason] = (reasons[reason] ?? 0) + 1;
      const breakpoint = call.cacheReuse.breakpoint;
      if (breakpoint) {
        breakpointKinds[breakpoint.kind] =
          (breakpointKinds[breakpoint.kind] ?? 0) + 1;
        breakpointChanges[breakpoint.change] =
          (breakpointChanges[breakpoint.change] ?? 0) + 1;
      }
    }
    const tail = calls.slice(-5);

    // Keep the task id internally to validate input shape without exposing it
    // in the aggregate output.
    if (!row[idField]) throw new Error(`Missing ${idField} in ${csvFile}`);
    return {
      id: row[idField]!,
      snapshot: row.snapshot ?? "unknown",
      calls: calls.map((call) => ({
        round: call.round,
        input: call.inputTokens,
        cached: call.cachedInputTokens,
      })),
      eligibleTransitions: eligible.length,
      broken: damaged.filter((call) => call.cacheReuse.state === "broken")
        .length,
      degraded: damaged.filter(
        (call) => call.cacheReuse.state === "degraded",
      ).length,
      reasons,
      breakpointKinds,
      breakpointChanges,
      tailInput: tail.reduce((sum, call) => sum + call.inputTokens, 0),
      tailCached: tail.reduce(
        (sum, call) => sum + call.cachedInputTokens,
        0,
      ),
      tailMedian: quantile(
        tail.map((call) => call.cachedInputTokens / call.inputTokens),
        0.5,
      ),
      tailP10: quantile(
        tail.map((call) => call.cachedInputTokens / call.inputTokens),
        0.1,
      ),
      tailP90: quantile(
        tail.map((call) => call.cachedInputTokens / call.inputTokens),
        0.9,
      ),
    };
  });
}

function summarizePairedSteadyState(runs: RunSummary[], minimumRound: number) {
  const before = new Map(
    runs
      .filter((run) => run.snapshot === "before")
      .map((run) => [run.id, run]),
  );
  const after = new Map(
    runs
      .filter((run) => run.snapshot === "after")
      .map((run) => [run.id, run]),
  );
  const pairedCalls = [...before.keys()].flatMap((id) => {
    const beforeRun = before.get(id);
    const afterRun = after.get(id);
    if (!beforeRun || !afterRun) return [];
    const afterByRound = new Map(
      afterRun.calls
        .filter((call) => call.round >= minimumRound)
        .map((call) => [call.round, call]),
    );
    return beforeRun.calls
      .filter((call) => call.round >= minimumRound)
      .flatMap((beforeCall) => {
        const afterCall = afterByRound.get(beforeCall.round);
        return afterCall ? [{ id, before: beforeCall, after: afterCall }] : [];
      });
  });
  const pairedRunCount = new Set(pairedCalls.map((pair) => pair.id)).size;

  return Object.fromEntries(
    (["before", "after"] as const).map((snapshot) => {
      const calls = pairedCalls.map((pair) => pair[snapshot]);
      const requestRates = calls.map((call) => call.cached / call.input);
      return [
        snapshot,
        {
          minimumRound,
          pairedRuns: pairedRunCount,
          pairedRequests: calls.length,
          requestMedianCacheHitRate: quantile(requestRates, 0.5),
          requestP10CacheHitRate: quantile(requestRates, 0.1),
          requestP90CacheHitRate: quantile(requestRates, 0.9),
          cacheHitRequestRate:
            calls.length === 0
              ? null
              : calls.filter((call) => call.cached > 0).length / calls.length,
          highReuseRequestRate:
            calls.length === 0
              ? null
              : requestRates.filter((rate) => rate >= 0.9).length /
                calls.length,
        },
      ];
    }),
  );
}

function summarize(runs: RunSummary[]) {
  return Object.fromEntries(
    ["before", "after"].map((snapshot) => {
      const selected = runs.filter((run) => run.snapshot === snapshot);
      const calls = selected.flatMap((run) => run.calls);
      const reasons: Record<string, number> = {};
      const breakpointKinds: Record<string, number> = {};
      const breakpointChanges: Record<string, number> = {};
      for (const run of selected) {
        for (const [reason, count] of Object.entries(run.reasons)) {
          reasons[reason] = (reasons[reason] ?? 0) + count;
        }
        for (const [kind, count] of Object.entries(run.breakpointKinds)) {
          breakpointKinds[kind] = (breakpointKinds[kind] ?? 0) + count;
        }
        for (const [change, count] of Object.entries(run.breakpointChanges)) {
          breakpointChanges[change] =
            (breakpointChanges[change] ?? 0) + count;
        }
      }
      const tailInput = selected.reduce((sum, run) => sum + run.tailInput, 0);
      const tailCached = selected.reduce(
        (sum, run) => sum + run.tailCached,
        0,
      );
      const roundBins = [
        [1, 2],
        [3, 5],
        [6, 10],
        [11, 20],
        [21, Number.POSITIVE_INFINITY],
      ].map(([low, high]) => {
        const entries = calls.filter(
          (call) => call.round >= low! && call.round <= high!,
        );
        const requestRates = entries.map((call) => call.cached / call.input);
        const input = entries.reduce((sum, call) => sum + call.input, 0);
        const cached = entries.reduce((sum, call) => sum + call.cached, 0);
        return {
          rounds: `${low}-${high === Number.POSITIVE_INFINITY ? "∞" : high}`,
          requests: entries.length,
          contributingRuns: selected.filter((run) =>
            run.calls.some(
              (call) => call.round >= low! && call.round <= high!,
            ),
          ).length,
          tokenWeightedCacheHitRate: input === 0 ? null : cached / input,
          requestMedianCacheHitRate: quantile(requestRates, 0.5),
          requestP10CacheHitRate: quantile(requestRates, 0.1),
          requestP90CacheHitRate: quantile(requestRates, 0.9),
          cacheHitRequestRate:
            entries.length === 0
              ? null
              : entries.filter((call) => call.cached > 0).length /
                entries.length,
          highReuseRequestRate:
            entries.length === 0
              ? null
              : requestRates.filter((rate) => rate >= 0.9).length /
                entries.length,
        };
      });

      return [
        snapshot,
        {
          runs: selected.length,
          usageReportedCalls: calls.length,
          eligibleTransitions: selected.reduce(
            (sum, run) => sum + run.eligibleTransitions,
            0,
          ),
          brokenTransitions: selected.reduce(
            (sum, run) => sum + run.broken,
            0,
          ),
          degradedTransitions: selected.reduce(
            (sum, run) => sum + run.degraded,
            0,
          ),
          breakReasons: reasons,
          breakpointKinds,
          breakpointChanges,
          tail5TokenWeightedCacheHitRate:
            tailInput === 0 ? null : tailCached / tailInput,
          tail5TaskP10CacheHitRate: quantile(
            selected
              .filter((run) => run.tailInput > 0)
              .map((run) => run.tailCached / run.tailInput),
            0.1,
          ),
          tail5TaskMedianCacheHitRate: quantile(
            selected
              .filter((run) => run.tailInput > 0)
              .map((run) => run.tailCached / run.tailInput),
            0.5,
          ),
          tail5TaskP90CacheHitRate: quantile(
            selected
              .filter((run) => run.tailInput > 0)
              .map((run) => run.tailCached / run.tailInput),
            0.9,
          ),
          roundBins,
        },
      ];
    }),
  );
}

const terminalCsv = requiredArgument("--terminal-csv");
const sweCsv = requiredArgument("--swe-csv");
const terminal = analyzeRuns(terminalCsv, "terminal", "task");
const swe = analyzeRuns(sweCsv, "swe", "instance_id");

console.log(
  JSON.stringify({
    terminal: summarize(terminal),
    terminalPairedSteadyState: summarizePairedSteadyState(terminal, 21),
    swe: summarize(swe),
    swePairedSteadyState: summarizePairedSteadyState(swe, 11),
  }),
);
