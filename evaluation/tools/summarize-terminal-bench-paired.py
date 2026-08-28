#!/usr/bin/env python3
"""Create a reproducible before/after summary from Harbor trial results.

The script consumes only result metadata and OpenTopia's content-free event
counts.  It never reads benchmark instructions, model text, tool arguments, or
tool output.  Failed or timed-out turns remain in the denominator for task
success, while unavailable efficiency telemetry is reported as missing rather
than silently converted to zero.
"""

from __future__ import annotations

import argparse
import csv
import json
import math
import statistics
from dataclasses import asdict, dataclass
from datetime import datetime
from pathlib import Path
from typing import Any, Iterable


@dataclass(frozen=True)
class Trial:
    task: str
    started_at: str | None
    finished_at: str | None
    agent_duration_seconds: float | None
    reward: float | None
    exception_type: str | None
    turn_status: str | None
    turn_error: str | None
    controlled_timeout: bool | None
    input_tokens: int | None
    cache_tokens: int | None
    output_tokens: int | None
    reasoning_tokens: int | None
    model_requests: int | None
    provider_responses: int | None
    tool_calls_started: int | None
    tool_calls_finished: int | None
    automatic_approvals: int | None
    event_count: int | None
    source_path: str


def parse_timestamp(value: Any) -> datetime | None:
    if not isinstance(value, str) or not value:
        return None
    try:
        return datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        return None


def duration_seconds(section: Any) -> float | None:
    if not isinstance(section, dict):
        return None
    start = parse_timestamp(section.get("started_at"))
    finish = parse_timestamp(section.get("finished_at"))
    if start is None or finish is None:
        return None
    return max(0.0, (finish - start).total_seconds())


def number(value: Any) -> int | None:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    return int(value)


def float_number(value: Any) -> float | None:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    return float(value)


def reward_from(result: Any) -> float | None:
    if not isinstance(result, dict):
        return None
    rewards = result.get("rewards")
    if not isinstance(rewards, dict):
        return None
    reward = rewards.get("reward")
    return float_number(reward)


def trial_from_file(path: Path) -> Trial | None:
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError):
        return None
    task = payload.get("task_name")
    if not isinstance(task, str) or not task:
        return None
    agent_result = payload.get("agent_result")
    if not isinstance(agent_result, dict):
        agent_result = {}
    metadata = agent_result.get("metadata")
    if not isinstance(metadata, dict):
        metadata = {}
    exception = payload.get("exception_info")
    if not isinstance(exception, dict):
        exception = {}
    return Trial(
        task=task,
        started_at=payload.get("started_at") if isinstance(payload.get("started_at"), str) else None,
        finished_at=payload.get("finished_at") if isinstance(payload.get("finished_at"), str) else None,
        agent_duration_seconds=duration_seconds(payload.get("agent_execution")),
        reward=reward_from(payload.get("verifier_result")),
        exception_type=exception.get("exception_type")
        if isinstance(exception.get("exception_type"), str)
        else None,
        turn_status=metadata.get("turnStatus")
        if isinstance(metadata.get("turnStatus"), str)
        else None,
        turn_error=metadata.get("turnError")
        if isinstance(metadata.get("turnError"), str)
        else None,
        controlled_timeout=metadata.get("controlledTimeout")
        if isinstance(metadata.get("controlledTimeout"), bool)
        else None,
        input_tokens=number(agent_result.get("n_input_tokens")),
        cache_tokens=number(agent_result.get("n_cache_tokens")),
        output_tokens=number(agent_result.get("n_output_tokens")),
        reasoning_tokens=number(metadata.get("reasoningTokens")),
        model_requests=number(metadata.get("modelRequests")),
        provider_responses=number(metadata.get("providerResponses")),
        tool_calls_started=number(metadata.get("toolCallsStarted")),
        tool_calls_finished=number(metadata.get("toolCallsFinished")),
        automatic_approvals=number(metadata.get("automaticApprovals")),
        event_count=number(metadata.get("eventCount")),
        source_path=str(path),
    )


def is_infrastructure_failure(trial: Trial) -> bool:
    """Identify terminal records that never exercised agent capability.

    Harbor only records adapter-raised failures in ``exception_info``.  A
    provider rejection can instead be represented as an otherwise complete
    agent result whose terminal turn failed before or during a model request.
    Those records have no benchmark meaning and must be retried rather than
    counted as a task failure in a paired comparison.
    """
    if trial.exception_type is not None:
        return True
    error = (trial.turn_error or "").casefold()
    # ``turnError`` is adapter-owned terminal metadata, not model/tool text.
    # Any explicit provider mention here therefore denotes an unavailable or
    # interrupted endpoint (including localized quota errors and a stream that
    # stalled after response headers), rather than a product outcome.
    if "provider" in error:
        return True
    return any(
        marker in error
        for marker in (
            "provider request failed",
            "provider stream returned an error",
            "error sending request",
            "error decoding response body",
            "upstream_error",
        )
    )


def latest_trials(roots: Iterable[Path]) -> dict[str, Trial]:
    """Select the latest non-infrastructure-failed trial for each task.

    Recovery directories may be supplied after a launcher fault.  An
    ``exception_info`` or a provider-request failure is not an agent evaluation
    and is therefore excluded in favour of a later completed retry for the
    same fixed snapshot.
    """
    candidates: dict[str, Trial] = {}
    for root in roots:
        for path in root.rglob("result.json"):
            # Harbor job-level results live directly below the timestamped job dir;
            # only nested task results carry a task_name.
            trial = trial_from_file(path)
            if trial is None or is_infrastructure_failure(trial):
                continue
            previous = candidates.get(trial.task)
            previous_time = parse_timestamp(previous.finished_at) if previous else None
            current_time = parse_timestamp(trial.finished_at)
            if previous is None or (
                current_time and (previous_time is None or current_time >= previous_time)
            ):
                candidates[trial.task] = trial
    return candidates


def mean(values: Iterable[int | float | None]) -> float | None:
    observed = [float(value) for value in values if value is not None]
    return sum(observed) / len(observed) if observed else None


def median(values: Iterable[int | float | None]) -> float | None:
    observed = [float(value) for value in values if value is not None]
    return float(statistics.median(observed)) if observed else None


def total(values: Iterable[int | float | None]) -> float | None:
    observed = [float(value) for value in values if value is not None]
    return sum(observed) if observed else None


def count_present(values: Iterable[Any]) -> int:
    return sum(value is not None for value in values)


def rate(numerator: int, denominator: int) -> float | None:
    return numerator / denominator if denominator else None


def summarize(trials: list[Trial]) -> dict[str, float | int | None]:
    count = len(trials)
    reward_observed = [trial.reward for trial in trials if trial.reward is not None]
    succeeded = sum(reward is not None and reward >= 1.0 for reward in reward_observed)
    input_total = total(trial.input_tokens for trial in trials)
    cache_total = total(trial.cache_tokens for trial in trials)
    uncached_by_trial = [
        trial.input_tokens - trial.cache_tokens
        for trial in trials
        if trial.input_tokens is not None and trial.cache_tokens is not None
    ]
    uncached_total = (
        input_total - cache_total
        if input_total is not None and cache_total is not None
        else None
    )
    return {
        "pairedTasks": count,
        "scoredTasks": len(reward_observed),
        "successes": succeeded,
        "taskSuccessRate": rate(succeeded, count),
        "rewardMean": mean(trial.reward for trial in trials),
        "runExceptions": sum(trial.exception_type is not None for trial in trials),
        "controlledTimeouts": sum(trial.controlled_timeout is True for trial in trials),
        "agentDurationMeanSeconds": mean(trial.agent_duration_seconds for trial in trials),
        "agentDurationMedianSeconds": median(trial.agent_duration_seconds for trial in trials),
        "inputTokensTotal": input_total,
        "inputTokensMean": mean(trial.input_tokens for trial in trials),
        "cacheTokensTotal": cache_total,
        "cacheTokensMean": mean(trial.cache_tokens for trial in trials),
        "cacheHitRate": (
            cache_total / input_total if input_total not in (None, 0) and cache_total is not None else None
        ),
        "uncachedInputTokensTotal": uncached_total,
        "uncachedInputTokensMean": mean(uncached_by_trial),
        "outputTokensTotal": total(trial.output_tokens for trial in trials),
        "outputTokensMean": mean(trial.output_tokens for trial in trials),
        "reasoningTokensTotal": total(trial.reasoning_tokens for trial in trials),
        "reasoningTokensMean": mean(trial.reasoning_tokens for trial in trials),
        "modelRequestsTotal": total(trial.model_requests for trial in trials),
        "modelRequestsMean": mean(trial.model_requests for trial in trials),
        "providerResponsesTotal": total(trial.provider_responses for trial in trials),
        "providerResponsesMean": mean(trial.provider_responses for trial in trials),
        "toolCallsStartedTotal": total(trial.tool_calls_started for trial in trials),
        "toolCallsStartedMean": mean(trial.tool_calls_started for trial in trials),
        "toolCallsFinishedTotal": total(trial.tool_calls_finished for trial in trials),
        "toolCallsFinishedMean": mean(trial.tool_calls_finished for trial in trials),
        "automaticApprovalsTotal": total(trial.automatic_approvals for trial in trials),
        "automaticApprovalsMean": mean(trial.automatic_approvals for trial in trials),
        "eventCountTotal": total(trial.event_count for trial in trials),
        "eventCountMean": mean(trial.event_count for trial in trials),
        "eventTelemetryCoverage": rate(
            count_present(trial.event_count for trial in trials), count
        ),
    }


def percent_delta(before: float | int | None, after: float | int | None) -> float | None:
    if before is None or after is None or before == 0:
        return None
    return (float(after) - float(before)) / float(before)


def display(value: Any, *, percentage: bool = False) -> str:
    if value is None:
        return "—"
    if percentage:
        return f"{float(value) * 100:.1f}%"
    if isinstance(value, int) or (isinstance(value, float) and value.is_integer()):
        return f"{int(value):,}"
    return f"{float(value):,.2f}"


def summary_markdown(before: dict[str, Any], after: dict[str, Any]) -> str:
    metrics = [
        ("任务成功率", "taskSuccessRate", True, "higher"),
        ("平均 agent 时长（秒）", "agentDurationMeanSeconds", False, "lower"),
        ("中位 agent 时长（秒）", "agentDurationMedianSeconds", False, "lower"),
        ("平均输入 token", "inputTokensMean", False, "lower"),
        ("平均缓存 token", "cacheTokensMean", False, "higher"),
        ("缓存命中率", "cacheHitRate", True, "higher"),
        ("平均未缓存输入 token", "uncachedInputTokensMean", False, "lower"),
        ("平均输出 token", "outputTokensMean", False, "lower"),
        ("平均推理 token", "reasoningTokensMean", False, "lower"),
        ("平均模型请求数", "modelRequestsMean", False, "lower"),
        ("平均模型响应数", "providerResponsesMean", False, "lower"),
        ("平均开始工具调用数", "toolCallsStartedMean", False, "neutral"),
        ("平均已完成工具调用数", "toolCallsFinishedMean", False, "neutral"),
        ("平均自动审批数", "automaticApprovalsMean", False, "neutral"),
        ("平均事件数", "eventCountMean", False, "neutral"),
        ("受控超时数", "controlledTimeouts", False, "lower"),
        ("运行异常数", "runExceptions", False, "lower"),
    ]
    lines = [
        "# Terminal-Bench before/after 汇总",
        "",
        "仅包含两个快照均有完成结果的任务；基础设施异常会由同一快照的恢复运行替代，缺失遥测不会伪造为零。",
        "",
        "| 指标 | Before | After | After 相对 Before | 方向 |",
        "|---|---:|---:|---:|---|",
    ]
    for label, key, percentage, direction in metrics:
        delta = percent_delta(before.get(key), after.get(key))
        lines.append(
            "| {label} | {before_value} | {after_value} | {delta} | {direction} |".format(
                label=label,
                before_value=display(before.get(key), percentage=percentage),
                after_value=display(after.get(key), percentage=percentage),
                delta=display(delta, percentage=True),
                direction={"higher": "越高越好", "lower": "越低越好", "neutral": "描述性"}[direction],
            )
        )
    lines.extend(
        [
            "",
            "## 覆盖度",
            "",
            f"- 配对任务：{before['pairedTasks']}",
            f"- Before 可评分任务：{before['scoredTasks']}；After 可评分任务：{after['scoredTasks']}",
            f"- Before 事件遥测覆盖：{display(before['eventTelemetryCoverage'], percentage=True)}；After：{display(after['eventTelemetryCoverage'], percentage=True)}",
            "",
        ]
    )
    return "\n".join(lines)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before-root", required=True, type=Path, action="append")
    parser.add_argument("--after-root", required=True, type=Path, action="append")
    parser.add_argument("--output-dir", required=True, type=Path)
    args = parser.parse_args()

    before_by_task = latest_trials(args.before_root)
    after_by_task = latest_trials(args.after_root)
    paired_tasks = sorted(set(before_by_task) & set(after_by_task))
    if not paired_tasks:
        raise SystemExit("No tasks were present in both before and after result roots.")

    before_trials = [before_by_task[task] for task in paired_tasks]
    after_trials = [after_by_task[task] for task in paired_tasks]
    before_summary = summarize(before_trials)
    after_summary = summarize(after_trials)

    args.output_dir.mkdir(parents=True, exist_ok=True)
    rows_path = args.output_dir / "paired-task-metrics.csv"
    fieldnames = ["snapshot"] + list(Trial.__dataclass_fields__)
    with rows_path.open("w", newline="", encoding="utf-8") as stream:
        writer = csv.DictWriter(stream, fieldnames=fieldnames)
        writer.writeheader()
        for snapshot, trials in (("before", before_trials), ("after", after_trials)):
            for trial in trials:
                writer.writerow({"task": trial.task, "snapshot": snapshot, **asdict(trial)})

    result = {
        "schemaVersion": 1,
        "beforeRoots": [str(root) for root in args.before_root],
        "afterRoots": [str(root) for root in args.after_root],
        "pairedTasks": paired_tasks,
        "before": before_summary,
        "after": after_summary,
    }
    (args.output_dir / "summary.json").write_text(
        json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    (args.output_dir / "summary.md").write_text(
        summary_markdown(before_summary, after_summary), encoding="utf-8"
    )


if __name__ == "__main__":
    main()
