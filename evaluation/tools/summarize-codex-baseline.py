#!/usr/bin/env python3
"""Summarize official Terminal-Bench and SWE-bench scores for local Codex."""

from __future__ import annotations

import argparse
import json
import statistics
from pathlib import Path
from typing import Any


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--terminal-root", type=Path, required=True)
    parser.add_argument("--swe-root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    return parser.parse_args()


def read_json(path: Path) -> dict[str, Any] | None:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    return value if isinstance(value, dict) else None


def terminal_entries(root: Path) -> list[dict[str, Any]]:
    newest: dict[str, tuple[float, dict[str, Any]]] = {}
    for path in root.rglob("result.json"):
        record = read_json(path)
        if not record or not isinstance(record.get("task_name"), str):
            continue
        task = record["task_name"]
        existing = newest.get(task)
        modified = path.stat().st_mtime
        if existing is None or modified > existing[0]:
            newest[task] = (modified, record)

    entries: list[dict[str, Any]] = []
    for task, (_, record) in sorted(newest.items()):
        agent = record.get("agent_result") or {}
        metadata = agent.get("metadata") or {}
        verifier = record.get("verifier_result") or {}
        rewards = verifier.get("rewards") or {}
        reward = rewards.get("reward")
        exception = record.get("exception_info")
        valid = isinstance(reward, (int, float)) and exception is None
        entries.append(
            {
                "task": task,
                "validOfficialResult": valid,
                "reward": float(reward) if isinstance(reward, (int, float)) else None,
                "turnStatus": metadata.get("turnStatus"),
                "controlledTimeout": metadata.get("controlledTimeout"),
                "durationSeconds": _number(metadata.get("durationSeconds")),
                "inputTokens": _integer(agent.get("n_input_tokens")),
                "cachedInputTokens": _integer(agent.get("n_cache_tokens")),
                "outputTokens": _integer(agent.get("n_output_tokens")),
                "reasoningTokens": _integer(metadata.get("reasoningTokens")),
                "commandExecutions": _integer(metadata.get("commandExecutionsFinished")),
                "errorType": (exception or {}).get("exception_type") if isinstance(exception, dict) else None,
            }
        )
    return entries


def swe_entries(root: Path) -> list[dict[str, Any]]:
    entries: list[dict[str, Any]] = []
    for run_path in sorted(root.rglob("prediction.run.json")):
        run = read_json(run_path)
        if not run or not isinstance(run.get("instanceId"), str):
            continue
        instance = run["instanceId"]
        score = read_json(run_path.parent / "official-score-lf-fixed.json")
        observation = _swe_observation(score, instance)
        controlled = run.get("controlledSettings") or {}
        usage = run.get("usage") or {}
        entries.append(
            {
                "instance": instance,
                "validOfficialResult": observation is not None,
                "resolved": observation.get("resolved") if observation else None,
                "patchApplied": observation.get("patch_successfully_applied") if observation else None,
                "infraFailure": observation.get("infra_failure") if observation else None,
                "turnStatus": controlled.get("turnStatus"),
                "controlledTimeout": controlled.get("controlledTimeout"),
                "durationSeconds": _number(run.get("agentDurationSeconds")),
                "inputTokens": _integer(usage.get("inputTokens")),
                "cachedInputTokens": _integer(usage.get("cachedInputTokens")),
                "outputTokens": _integer(usage.get("outputTokens")),
                "reasoningTokens": _integer(usage.get("reasoningTokens")),
                "commandExecutions": _integer(controlled.get("commandExecutionsFinished")),
                "runStatus": run.get("status"),
            }
        )
    return entries


def _swe_observation(score: dict[str, Any] | None, instance: str) -> dict[str, Any] | None:
    if not score:
        return None
    result = score.get("result")
    if not isinstance(result, list) or len(result) < 2 or not isinstance(result[1], dict):
        return None
    observation = result[1].get(instance)
    return observation if isinstance(observation, dict) else None


def _number(value: Any) -> float | None:
    return float(value) if isinstance(value, (int, float)) else None


def _integer(value: Any) -> int:
    return int(value) if isinstance(value, (int, float)) else 0


def aggregate(entries: list[dict[str, Any]], *, success_key: str) -> dict[str, Any]:
    valid = [entry for entry in entries if entry["validOfficialResult"]]
    successes = sum(entry.get(success_key) is True or entry.get(success_key) == 1.0 for entry in valid)
    numeric_fields = [
        "durationSeconds",
        "inputTokens",
        "cachedInputTokens",
        "outputTokens",
        "reasoningTokens",
        "commandExecutions",
    ]
    summary: dict[str, Any] = {
        "scheduled": len(entries),
        "validOfficialResults": len(valid),
        "successes": successes,
        "successRate": successes / len(valid) if valid else None,
        "controlledTimeouts": sum(entry.get("controlledTimeout") is True for entry in valid),
    }
    for field in numeric_fields:
        values = [float(entry[field]) for entry in valid if isinstance(entry.get(field), (int, float))]
        summary[f"{field}Total"] = sum(values)
        summary[f"{field}Mean"] = statistics.mean(values) if values else None
    input_total = summary["inputTokensTotal"]
    cache_total = summary["cachedInputTokensTotal"]
    summary["cacheHitRate"] = cache_total / input_total if input_total else None
    return summary


def main() -> None:
    args = parse_args()
    terminal = terminal_entries(args.terminal_root)
    swe = swe_entries(args.swe_root)
    report = {
        "schemaVersion": 1,
        "modelSelection": "codex-account-default-no-model-flag",
        "terminalBench": {"summary": aggregate(terminal, success_key="reward"), "tasks": terminal},
        "sweBenchVerified": {"summary": aggregate(swe, success_key="resolved"), "instances": swe},
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
