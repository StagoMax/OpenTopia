#!/usr/bin/env python3
"""Summarize paired OpenTopia SWE-bench runs without reading patch/event contents."""

from __future__ import annotations

import argparse
import csv
import json
import statistics
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any, Iterable


@dataclass(frozen=True)
class Trial:
    instance_id: str
    source_path: str
    agent_status: str | None
    turn_status: str | None
    controlled_timeout: bool | None
    agent_duration_seconds: float | None
    resolved: bool | None
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
    patch_bytes: int | None
    cache_telemetry: str | None


def integer(value: Any) -> int | None:
    return int(value) if isinstance(value, (int, float)) and not isinstance(value, bool) else None


def number(value: Any) -> float | None:
    return float(value) if isinstance(value, (int, float)) and not isinstance(value, bool) else None


def mean(values: Iterable[int | float | None]) -> float | None:
    observed = [float(value) for value in values if value is not None]
    return statistics.mean(observed) if observed else None


def median(values: Iterable[int | float | None]) -> float | None:
    observed = [float(value) for value in values if value is not None]
    return statistics.median(observed) if observed else None


def total(values: Iterable[int | float | None]) -> int | float | None:
    observed = [value for value in values if value is not None]
    return sum(observed) if observed else None


def rate(numerator: int, denominator: int) -> float | None:
    return numerator / denominator if denominator else None


def resolved_from_score(payload: Any, instance_id: str) -> bool | None:
    """Read the public SWE-bench `run_instance` return shape, not test output."""
    if not isinstance(payload, dict):
        return None
    result = payload.get("result")
    # `run_instance` returns `(instance_id, {instance_id: {resolved: bool, ...}})`.
    if not isinstance(result, list) or len(result) != 2 or result[0] != instance_id:
        return None
    report = result[1]
    if not isinstance(report, dict):
        return None
    entry = report.get(instance_id)
    if not isinstance(entry, dict) or not isinstance(entry.get("resolved"), bool):
        return None
    return entry["resolved"]


def provider_failure(settings: dict[str, Any]) -> bool:
    error = settings.get("turnError")
    if not isinstance(error, str):
        return False
    normalized = error.lower()
    # This field is written by the adapter.  A provider-labelled terminal
    # error—whether localized quota text or a stalled stream—is infrastructure,
    # never a valid agent/official-score outcome.
    if "provider" in normalized:
        return True
    return any(
        marker in normalized
        for marker in (
            "provider request failed",
            "provider stream returned an error",
            "error sending request",
            "error decoding response body",
            "upstream_error",
        )
    )


def read_trial(path: Path) -> Trial | None:
    record = json.loads(path.read_text(encoding="utf-8"))
    instance_id = record.get("instanceId")
    if not isinstance(instance_id, str):
        return None
    # Infrastructure failures have no trustworthy agent/score outcome and must
    # be excluded rather than being silently converted to zero-valued metrics.
    if record.get("status") == "infrastructure_error":
        return None
    settings = record.get("controlledSettings")
    settings = settings if isinstance(settings, dict) else {}
    if provider_failure(settings):
        return None
    # A LF-preserving rescore supersedes the original Windows text-mode score.
    # Keep the original artifact for audit, but prefer the corrected official
    # result when it exists.
    score_path = path.parent / "official-score-lf-fixed.json"
    if not score_path.is_file():
        score_path = path.parent / "official-score.json"
    if not score_path.is_file():
        return None
    resolved = resolved_from_score(
        json.loads(score_path.read_text(encoding="utf-8")), instance_id
    )
    if resolved is None:
        return None

    usage = record.get("usage")
    usage = usage if isinstance(usage, dict) else {}
    return Trial(
        instance_id=instance_id,
        source_path=str(path),
        agent_status=record.get("status") if isinstance(record.get("status"), str) else None,
        turn_status=settings.get("turnStatus") if isinstance(settings.get("turnStatus"), str) else None,
        controlled_timeout=settings.get("controlledTimeout") if isinstance(settings.get("controlledTimeout"), bool) else None,
        agent_duration_seconds=number(record.get("agentDurationSeconds")),
        resolved=resolved,
        input_tokens=integer(usage.get("inputTokens")),
        cache_tokens=integer(usage.get("cachedInputTokens")),
        output_tokens=integer(usage.get("outputTokens")),
        reasoning_tokens=integer(usage.get("reasoningTokens")),
        model_requests=integer(settings.get("modelRequests")),
        provider_responses=integer(settings.get("providerResponses")),
        tool_calls_started=integer(settings.get("toolCallsStarted")),
        tool_calls_finished=integer(settings.get("toolCallsFinished")),
        automatic_approvals=integer(settings.get("automaticApprovals")),
        event_count=integer(record.get("eventCount")),
        patch_bytes=integer(record.get("patchBytes")),
        cache_telemetry=record.get("cacheTelemetry") if isinstance(record.get("cacheTelemetry"), str) else None,
    )


def latest_trials(roots: list[Path]) -> dict[str, Trial]:
    trials: dict[str, Trial] = {}
    for root in roots:
        for path in root.rglob("*.run.json"):
            trial = read_trial(path)
            if trial is None:
                continue
            existing = trials.get(trial.instance_id)
            if existing is None or path.stat().st_mtime > Path(existing.source_path).stat().st_mtime:
                trials[trial.instance_id] = trial
    return trials


def summarize(trials: list[Trial]) -> dict[str, int | float | None]:
    count = len(trials)
    input_total = total(trial.input_tokens for trial in trials)
    cache_total = total(trial.cache_tokens for trial in trials)
    uncached = [
        trial.input_tokens - trial.cache_tokens
        for trial in trials
        if trial.input_tokens is not None and trial.cache_tokens is not None
    ]
    return {
        "pairedInstances": count,
        "resolved": sum(trial.resolved is True for trial in trials),
        "resolveRate": rate(sum(trial.resolved is True for trial in trials), count),
        "agentFailures": sum(trial.agent_status != "completed" for trial in trials),
        "controlledTimeouts": sum(trial.controlled_timeout is True for trial in trials),
        "agentDurationMeanSeconds": mean(trial.agent_duration_seconds for trial in trials),
        "agentDurationMedianSeconds": median(trial.agent_duration_seconds for trial in trials),
        "inputTokensTotal": input_total,
        "inputTokensMean": mean(trial.input_tokens for trial in trials),
        "cacheTokensTotal": cache_total,
        "cacheTokensMean": mean(trial.cache_tokens for trial in trials),
        "cacheHitRate": cache_total / input_total if input_total not in (None, 0) and cache_total is not None else None,
        "uncachedInputTokensTotal": input_total - cache_total if input_total is not None and cache_total is not None else None,
        "uncachedInputTokensMean": mean(uncached),
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
        "patchBytesTotal": total(trial.patch_bytes for trial in trials),
        "patchBytesMean": mean(trial.patch_bytes for trial in trials),
        "providerCacheTelemetryCoverage": rate(
            sum(trial.cache_telemetry == "provider_reported" for trial in trials), count
        ),
    }


def percent_delta(before: int | float | None, after: int | float | None) -> float | None:
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


def markdown(before: dict[str, Any], after: dict[str, Any]) -> str:
    metrics = [
        ("官方 resolved 率", "resolveRate", True, "越高越好"),
        ("平均 agent 时长（秒）", "agentDurationMeanSeconds", False, "越低越好"),
        ("中位 agent 时长（秒）", "agentDurationMedianSeconds", False, "越低越好"),
        ("平均输入 token", "inputTokensMean", False, "越低越好"),
        ("平均缓存 token", "cacheTokensMean", False, "越高越好"),
        ("缓存命中率", "cacheHitRate", True, "越高越好"),
        ("平均未缓存输入 token", "uncachedInputTokensMean", False, "越低越好"),
        ("平均输出 token", "outputTokensMean", False, "越低越好"),
        ("平均推理 token", "reasoningTokensMean", False, "越低越好"),
        ("平均模型请求数", "modelRequestsMean", False, "越低越好"),
        ("平均模型响应数", "providerResponsesMean", False, "越低越好"),
        ("平均开始工具调用数", "toolCallsStartedMean", False, "描述性"),
        ("平均已完成工具调用数", "toolCallsFinishedMean", False, "描述性"),
        ("平均自动审批数", "automaticApprovalsMean", False, "描述性"),
        ("平均事件数", "eventCountMean", False, "描述性"),
        ("平均 patch 字节数", "patchBytesMean", False, "描述性"),
        ("受控超时数", "controlledTimeouts", False, "越低越好"),
    ]
    lines = [
        "# SWE-bench Verified before/after 汇总",
        "",
        "仅包含 before 与 after 均有官方 SWE-bench 5.x resolved 结果的实例；不把基础设施错误或缺失遥测当作零值。",
        "",
        "| 指标 | Before | After | After 相对 Before | 方向 |",
        "|---|---:|---:|---:|---|",
    ]
    for label, key, percentage, direction in metrics:
        lines.append(
            f"| {label} | {display(before.get(key), percentage=percentage)} | "
            f"{display(after.get(key), percentage=percentage)} | "
            f"{display(percent_delta(before.get(key), after.get(key)), percentage=True)} | {direction} |"
        )
    lines.extend([
        "",
        f"- 配对实例：{before['pairedInstances']}；Before resolved：{before['resolved']}；After resolved：{after['resolved']}",
        f"- Before provider 缓存遥测覆盖：{display(before['providerCacheTelemetryCoverage'], percentage=True)}；After：{display(after['providerCacheTelemetryCoverage'], percentage=True)}",
        "",
    ])
    return "\n".join(lines)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before-root", required=True, type=Path, action="append")
    parser.add_argument("--after-root", required=True, type=Path, action="append")
    parser.add_argument("--output-dir", required=True, type=Path)
    args = parser.parse_args()

    before_by_instance = latest_trials(args.before_root)
    after_by_instance = latest_trials(args.after_root)
    paired = sorted(set(before_by_instance) & set(after_by_instance))
    if not paired:
        raise SystemExit("No officially scored instances were present in both roots.")
    before_trials = [before_by_instance[instance] for instance in paired]
    after_trials = [after_by_instance[instance] for instance in paired]

    args.output_dir.mkdir(parents=True, exist_ok=True)
    rows_path = args.output_dir / "paired-instance-metrics.csv"
    with rows_path.open("w", newline="", encoding="utf-8") as stream:
        writer = csv.DictWriter(stream, fieldnames=["snapshot"] + list(Trial.__dataclass_fields__))
        writer.writeheader()
        for snapshot, trials in (("before", before_trials), ("after", after_trials)):
            for trial in trials:
                writer.writerow({"snapshot": snapshot, **asdict(trial)})

    before = summarize(before_trials)
    after = summarize(after_trials)
    result = {
        "schemaVersion": 1,
        "beforeRoots": [str(root) for root in args.before_root],
        "afterRoots": [str(root) for root in args.after_root],
        "pairedInstances": paired,
        "before": before,
        "after": after,
    }
    (args.output_dir / "summary.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    (args.output_dir / "summary.md").write_text(markdown(before, after), encoding="utf-8")


if __name__ == "__main__":
    main()
