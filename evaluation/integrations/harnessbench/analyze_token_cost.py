"""Audit paired Harness-Bench token accounting without making API calls."""

from __future__ import annotations

import argparse
import json
import statistics
from collections import defaultdict
from pathlib import Path
from typing import Any


PRICE_PER_MILLION = {
    "uncached_input": 0.50,
    "cached_input": 0.05,
    "output": 3.00,
}

BREAKDOWN_KEYS = (
    "baseInstructions",
    "developerInstructions",
    "repositoryInstructions",
    "runtimeContext",
    "skillInstructions",
    "summaries",
    "checkpoints",
    "conversation",
    "currentUser",
    "toolCalls",
    "toolResults",
    "toolSchemas",
    "outputSchema",
    "turnAssistantState",
    "providerState",
    "other",
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("status", type=Path)
    return parser.parse_args()


def read_jsonl(path: Path) -> list[dict[str, Any]]:
    if not path.is_file():
        return []
    rows: list[dict[str, Any]] = []
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        if not line.strip():
            continue
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict):
            rows.append(value)
    return rows


def proxy_rows(row: dict[str, Any]) -> list[dict[str, Any]]:
    return read_jsonl(Path(row["sandbox"]) / "usage-proxy" / "requests.jsonl")


def event_usage_rows(row: dict[str, Any]) -> list[dict[str, Any]]:
    events = raw_event_rows(row)
    return [
        dict(event.get("payload") or {})
        for event in events
        if event.get("type") == "model.usage"
        and isinstance(event.get("payload"), dict)
    ]


def raw_event_rows(row: dict[str, Any]) -> list[dict[str, Any]]:
    return read_jsonl(Path(row["sandbox"]) / "opentopia" / "events.jsonl")


def failed_tool_call(event: dict[str, Any]) -> dict[str, str] | None:
    if event.get("type") != "tool.call.completed":
        return None
    payload = event.get("payload") or {}
    if not isinstance(payload, dict) or payload.get("success") is not False:
        return None
    result = payload.get("result") or {}
    metadata = result.get("metadata") or {} if isinstance(result, dict) else {}
    record = metadata.get("errorRecord") or {} if isinstance(metadata, dict) else {}
    text_parts = [
        result.get("output") if isinstance(result, dict) else None,
        metadata.get("error") if isinstance(metadata, dict) else None,
        record.get("message") if isinstance(record, dict) else None,
    ]
    error_text = "\n".join(str(part) for part in text_parts if part)
    code = str(record.get("code") or "unknown") if isinstance(record, dict) else "unknown"
    if "turn file mutation arrived after capture finalized" in error_text:
        failure_class = "mutation_journal_after_capture_finalized"
    elif code == "invalid_tool_arguments":
        failure_class = "invalid_tool_arguments"
    elif code == "command_exit_nonzero":
        failure_class = "command_exit_nonzero"
    else:
        failure_class = f"other:{code}"
    return {
        "tool": str(payload.get("name") or metadata.get("toolName") or "unknown"),
        "code": code,
        "class": failure_class,
    }


def usage(row: dict[str, Any]) -> dict[str, int]:
    uncached = int(row.get("input_tokens") or 0)
    cached = int(row.get("cache_read_tokens") or 0)
    output = int(row.get("output_tokens") or 0)
    return {
        "uncached_input_tokens": uncached,
        "cached_input_tokens": cached,
        "total_input_tokens": uncached + cached,
        "output_tokens": output,
        "total_tokens": int(row.get("total_tokens") or 0),
    }


def add_usage(target: dict[str, int], source: dict[str, int]) -> None:
    for key, value in source.items():
        target[key] += int(value)


def empty_usage() -> dict[str, int]:
    return {
        "uncached_input_tokens": 0,
        "cached_input_tokens": 0,
        "total_input_tokens": 0,
        "output_tokens": 0,
        "total_tokens": 0,
    }


def cost_components(tokens: dict[str, int]) -> dict[str, float]:
    uncached = (
        tokens["uncached_input_tokens"]
        * PRICE_PER_MILLION["uncached_input"]
        / 1_000_000
    )
    cached = (
        tokens["cached_input_tokens"]
        * PRICE_PER_MILLION["cached_input"]
        / 1_000_000
    )
    output = tokens["output_tokens"] * PRICE_PER_MILLION["output"] / 1_000_000
    return {
        "uncached_input_cny": uncached,
        "cached_input_cny": cached,
        "output_cny": output,
        "total_cny": uncached + cached + output,
    }


def quantiles(values: list[int]) -> dict[str, float | int | None]:
    if not values:
        return {"mean": None, "median": None, "p90": None, "max": None}
    ordered = sorted(values)
    p90_index = min(len(ordered) - 1, max(0, int(0.9 * len(ordered)) - 1))
    return {
        "mean": statistics.fmean(values),
        "median": statistics.median(values),
        "p90": ordered[p90_index],
        "max": max(values),
    }


def main() -> int:
    args = parse_args()
    status = json.loads(args.status.read_text(encoding="utf-8"))
    valid = [row for row in status.get("attempts", []) if row.get("valid") is True]
    by_key = {(row["snapshot"], row["task_id"]): row for row in valid}
    task_ids = sorted(
        task_id
        for task_id in {row["task_id"] for row in valid}
        if ("before", task_id) in by_key and ("after", task_id) in by_key
    )

    proxy_by_key: dict[tuple[str, str], list[dict[str, Any]]] = {}
    events_by_key: dict[tuple[str, str], list[dict[str, Any]]] = {}
    raw_events_by_key: dict[tuple[str, str], list[dict[str, Any]]] = {}
    proxy_totals = {"before": empty_usage(), "after": empty_usage()}
    status_totals = {"before": empty_usage(), "after": empty_usage()}
    accounting_mismatches: list[dict[str, Any]] = []
    request_counts: dict[str, list[int]] = {"before": [], "after": []}

    for snapshot in ("before", "after"):
        for task_id in task_ids:
            row = by_key[(snapshot, task_id)]
            rows = proxy_rows(row)
            event_rows = event_usage_rows(row)
            raw_events = raw_event_rows(row)
            proxy_by_key[(snapshot, task_id)] = rows
            events_by_key[(snapshot, task_id)] = event_rows
            raw_events_by_key[(snapshot, task_id)] = raw_events
            request_counts[snapshot].append(len(rows))

            task_proxy_total = empty_usage()
            for request_index, request in enumerate(rows, start=1):
                current = usage(request)
                add_usage(task_proxy_total, current)
                expected_total = (
                    current["uncached_input_tokens"]
                    + current["cached_input_tokens"]
                    + current["output_tokens"]
                )
                if expected_total != current["total_tokens"]:
                    accounting_mismatches.append(
                        {
                            "snapshot": snapshot,
                            "task_id": task_id,
                            "request": request_index,
                            "expected_total": expected_total,
                            "reported_total": current["total_tokens"],
                        }
                    )
            add_usage(proxy_totals[snapshot], task_proxy_total)

            saved = row.get("usage") or {}
            task_status_total = {
                "uncached_input_tokens": int(saved.get("uncached_input_tokens") or 0),
                "cached_input_tokens": int(saved.get("cached_input_tokens") or 0),
                "total_input_tokens": int(saved.get("total_input_tokens") or 0),
                "output_tokens": int(saved.get("output_tokens") or 0),
                "total_tokens": int(saved.get("total_tokens") or 0),
            }
            add_usage(status_totals[snapshot], task_status_total)
            if task_proxy_total != task_status_total:
                accounting_mismatches.append(
                    {
                        "snapshot": snapshot,
                        "task_id": task_id,
                        "kind": "status_vs_proxy",
                        "proxy": task_proxy_total,
                        "status": task_status_total,
                    }
                )

    common = {"before": empty_usage(), "after": empty_usage()}
    extra = {"before": empty_usage(), "after": empty_usage()}
    same_count = {"before": empty_usage(), "after": empty_usage()}
    same_count_tasks = 0
    first_request = {"before": empty_usage(), "after": empty_usage()}
    task_cost_deltas: list[dict[str, Any]] = []

    for task_id in task_ids:
        before_rows = proxy_by_key[("before", task_id)]
        after_rows = proxy_by_key[("after", task_id)]
        common_count = min(len(before_rows), len(after_rows))
        for snapshot, rows in (("before", before_rows), ("after", after_rows)):
            if rows:
                add_usage(first_request[snapshot], usage(rows[0]))
            for request in rows[:common_count]:
                add_usage(common[snapshot], usage(request))
            for request in rows[common_count:]:
                add_usage(extra[snapshot], usage(request))
        if len(before_rows) == len(after_rows):
            same_count_tasks += 1
            for snapshot, rows in (("before", before_rows), ("after", after_rows)):
                for request in rows:
                    add_usage(same_count[snapshot], usage(request))

        before_tokens = empty_usage()
        after_tokens = empty_usage()
        for request in before_rows:
            add_usage(before_tokens, usage(request))
        for request in after_rows:
            add_usage(after_tokens, usage(request))
        before_cost = cost_components(before_tokens)["total_cny"]
        after_cost = cost_components(after_tokens)["total_cny"]
        task_cost_deltas.append(
            {
                "task_id": task_id,
                "before_requests": len(before_rows),
                "after_requests": len(after_rows),
                "before_cost_cny": before_cost,
                "after_cost_cny": after_cost,
                "delta_cny": after_cost - before_cost,
                "oracle_delta": float(
                    by_key[("after", task_id)].get("oracle_outcome_score") or 0.0
                )
                - float(
                    by_key[("before", task_id)].get("oracle_outcome_score") or 0.0
                ),
            }
        )

    failure_counts: dict[str, dict[str, int]] = {
        "before": defaultdict(int),
        "after": defaultdict(int),
    }
    failure_tool_counts: dict[str, dict[str, dict[str, int]]] = {
        "before": defaultdict(lambda: defaultdict(int)),
        "after": defaultdict(lambda: defaultdict(int)),
    }
    affected_tasks: dict[str, dict[str, set[str]]] = {
        "before": defaultdict(set),
        "after": defaultdict(set),
    }
    post_first_failure = {
        "before": {"requests": 0, "tokens": empty_usage(), "tasks": 0},
        "after": {"requests": 0, "tokens": empty_usage(), "tasks": 0},
    }
    direct_failure_followups = {
        "before": {"requests": 0, "tokens": empty_usage(), "tasks": set()},
        "after": {"requests": 0, "tokens": empty_usage(), "tasks": set()},
    }
    task_failure_classes: dict[tuple[str, str], set[str]] = {}

    for snapshot in ("before", "after"):
        for task_id in task_ids:
            model_requests_seen = 0
            first_failure_after_request: int | None = None
            failures_since_last_model_request = False
            classes: set[str] = set()
            for event in raw_events_by_key[(snapshot, task_id)]:
                if event.get("type") == "model.usage":
                    if failures_since_last_model_request:
                        request_index = model_requests_seen
                        requests = proxy_by_key[(snapshot, task_id)]
                        if request_index < len(requests):
                            direct_failure_followups[snapshot]["requests"] += 1
                            direct_failure_followups[snapshot]["tasks"].add(task_id)
                            add_usage(
                                direct_failure_followups[snapshot]["tokens"],
                                usage(requests[request_index]),
                            )
                        failures_since_last_model_request = False
                    model_requests_seen += 1
                    continue
                failure = failed_tool_call(event)
                if failure is None:
                    continue
                if first_failure_after_request is None:
                    first_failure_after_request = model_requests_seen
                failures_since_last_model_request = True
                failure_class = failure["class"]
                classes.add(failure_class)
                failure_counts[snapshot][failure_class] += 1
                failure_tool_counts[snapshot][failure_class][failure["tool"]] += 1
                affected_tasks[snapshot][failure_class].add(task_id)
            task_failure_classes[(snapshot, task_id)] = classes
            if first_failure_after_request is not None:
                post_first_failure[snapshot]["tasks"] += 1
                later_requests = proxy_by_key[(snapshot, task_id)][
                    first_failure_after_request:
                ]
                post_first_failure[snapshot]["requests"] += len(later_requests)
                for request in later_requests:
                    add_usage(post_first_failure[snapshot]["tokens"], usage(request))

    affected_group_stats: dict[str, dict[str, Any]] = {}
    after_classes = sorted(failure_counts["after"])
    for failure_class in after_classes:
        class_tasks = affected_tasks["after"][failure_class]
        rows = [row for row in task_cost_deltas if row["task_id"] in class_tasks]
        affected_group_stats[failure_class] = {
            "tasks": len(class_tasks),
            "failures": failure_counts["after"][failure_class],
            "by_tool": dict(sorted(failure_tool_counts["after"][failure_class].items())),
            "total_request_delta": sum(
                row["after_requests"] - row["before_requests"] for row in rows
            ),
            "mean_request_delta": statistics.fmean(
                row["after_requests"] - row["before_requests"] for row in rows
            )
            if rows
            else 0.0,
            "total_cost_delta_cny": sum(row["delta_cny"] for row in rows),
            "mean_oracle_delta": statistics.fmean(row["oracle_delta"] for row in rows)
            if rows
            else 0.0,
        }

    journal_tasks = affected_tasks["after"].get(
        "mutation_journal_after_capture_finalized", set()
    )
    journal_partition: dict[str, dict[str, Any]] = {}
    for label, selected in (
        ("journal_affected", journal_tasks),
        ("not_journal_affected", set(task_ids) - journal_tasks),
    ):
        rows = [row for row in task_cost_deltas if row["task_id"] in selected]
        journal_partition[label] = {
            "tasks": len(rows),
            "total_request_delta": sum(
                row["after_requests"] - row["before_requests"] for row in rows
            ),
            "mean_request_delta": statistics.fmean(
                row["after_requests"] - row["before_requests"] for row in rows
            )
            if rows
            else 0.0,
            "total_cost_delta_cny": sum(row["delta_cny"] for row in rows),
            "mean_oracle_delta": statistics.fmean(row["oracle_delta"] for row in rows)
            if rows
            else 0.0,
        }

    breakdown_sums = {
        "first_request": {"before": defaultdict(int), "after": defaultdict(int)},
        "common_requests": {"before": defaultdict(int), "after": defaultdict(int)},
    }
    breakdown_counts = {
        "first_request": {"before": 0, "after": 0},
        "common_requests": {"before": 0, "after": 0},
    }

    for task_id in task_ids:
        before_events = events_by_key[("before", task_id)]
        after_events = events_by_key[("after", task_id)]
        common_count = min(len(before_events), len(after_events))
        for snapshot, rows in (("before", before_events), ("after", after_events)):
            selected_sets = {
                "first_request": rows[:1],
                "common_requests": rows[:common_count],
            }
            for label, selected in selected_sets.items():
                for event in selected:
                    breakdown = event.get("inputBreakdown") or {}
                    if not isinstance(breakdown, dict):
                        continue
                    breakdown_counts[label][snapshot] += 1
                    for key in BREAKDOWN_KEYS:
                        breakdown_sums[label][snapshot][key] += int(
                            breakdown.get(key) or 0
                        )

    breakdown_means: dict[str, dict[str, dict[str, float]]] = {}
    for label in breakdown_sums:
        breakdown_means[label] = {}
        for snapshot in ("before", "after"):
            count = breakdown_counts[label][snapshot]
            breakdown_means[label][snapshot] = {
                key: (breakdown_sums[label][snapshot][key] / count if count else 0.0)
                for key in BREAKDOWN_KEYS
            }

    total_costs = {
        snapshot: cost_components(proxy_totals[snapshot])
        for snapshot in ("before", "after")
    }
    common_costs = {
        snapshot: cost_components(common[snapshot])
        for snapshot in ("before", "after")
    }
    extra_costs = {
        snapshot: cost_components(extra[snapshot])
        for snapshot in ("before", "after")
    }

    result = {
        "paired_tasks": len(task_ids),
        "accounting_validation": {
            "mismatch_count": len(accounting_mismatches),
            "mismatches": accounting_mismatches[:20],
            "status_totals": status_totals,
            "proxy_totals": proxy_totals,
        },
        "request_counts": {
            snapshot: {
                "total": sum(request_counts[snapshot]),
                **quantiles(request_counts[snapshot]),
            }
            for snapshot in ("before", "after")
        },
        "total": {
            snapshot: {
                "tokens": proxy_totals[snapshot],
                "cost": total_costs[snapshot],
            }
            for snapshot in ("before", "after")
        },
        "cost_delta_components_cny": {
            key: total_costs["after"][key] - total_costs["before"][key]
            for key in total_costs["before"]
        },
        "common_request_prefix": {
            snapshot: {
                "tokens": common[snapshot],
                "cost": common_costs[snapshot],
            }
            for snapshot in ("before", "after")
        },
        "extra_requests_beyond_other_snapshot": {
            snapshot: {
                "tokens": extra[snapshot],
                "cost": extra_costs[snapshot],
            }
            for snapshot in ("before", "after")
        },
        "same_request_count_subset": {
            "tasks": same_count_tasks,
            "before": {
                "tokens": same_count["before"],
                "cost": cost_components(same_count["before"]),
            },
            "after": {
                "tokens": same_count["after"],
                "cost": cost_components(same_count["after"]),
            },
        },
        "first_request": {
            snapshot: {
                "tokens": first_request[snapshot],
                "cost": cost_components(first_request[snapshot]),
            }
            for snapshot in ("before", "after")
        },
        "local_input_breakdown_mean": breakdown_means,
        "tool_failure_analysis": {
            "counts": {
                snapshot: dict(sorted(failure_counts[snapshot].items()))
                for snapshot in ("before", "after")
            },
            "affected_task_counts": {
                snapshot: {
                    failure_class: len(tasks)
                    for failure_class, tasks in sorted(
                        affected_tasks[snapshot].items()
                    )
                }
                for snapshot in ("before", "after")
            },
            "after_groups": affected_group_stats,
            "journal_partition": journal_partition,
            "post_first_failure": {
                snapshot: {
                    "tasks": post_first_failure[snapshot]["tasks"],
                    "requests": post_first_failure[snapshot]["requests"],
                    "tokens": post_first_failure[snapshot]["tokens"],
                    "cost": cost_components(post_first_failure[snapshot]["tokens"]),
                }
                for snapshot in ("before", "after")
            },
            "direct_failure_followups": {
                snapshot: {
                    "tasks": len(direct_failure_followups[snapshot]["tasks"]),
                    "requests": direct_failure_followups[snapshot]["requests"],
                    "tokens": direct_failure_followups[snapshot]["tokens"],
                    "cost": cost_components(
                        direct_failure_followups[snapshot]["tokens"]
                    ),
                }
                for snapshot in ("before", "after")
            },
        },
        "largest_task_cost_increases": sorted(
            task_cost_deltas, key=lambda item: item["delta_cny"], reverse=True
        )[:15],
        "largest_task_cost_decreases": sorted(
            task_cost_deltas, key=lambda item: item["delta_cny"]
        )[:15],
    }
    print(json.dumps(result, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
