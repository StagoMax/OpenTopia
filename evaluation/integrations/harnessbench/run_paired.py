"""Run a paired Harness-Bench pilot against two shared OpenTopia servers."""

from __future__ import annotations

import argparse
import json
import os
import statistics
import subprocess
import sys
import threading
import time
import traceback
from concurrent.futures import Future, ThreadPoolExecutor, as_completed
from dataclasses import asdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


def _arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--harness-root", type=Path, required=True)
    parser.add_argument("--output-root", type=Path, required=True)
    parser.add_argument("--work-root", type=Path, required=True)
    parser.add_argument("--selection", type=Path, required=True)
    parser.add_argument("--status-file", default="pilot-status.json")
    parser.add_argument("--reuse-before-status", type=Path)
    parser.add_argument("--reuse-before-provenance", type=Path)
    parser.add_argument("--summary-file", default="pilot-summary.json")
    parser.add_argument("--report-file", default="第一阶段试点报告.md")
    parser.add_argument("--task-ids", default="")
    parser.add_argument("--before-url", required=True)
    parser.add_argument("--after-url", required=True)
    parser.add_argument("--before-provider", default="default")
    parser.add_argument("--after-provider", default="default")
    parser.add_argument("--before-artifact-sha256", required=True)
    parser.add_argument("--after-artifact-sha256", required=True)
    parser.add_argument("--model", default="gpt-5.6-terra")
    parser.add_argument("--reasoning-effort", default="high")
    parser.add_argument("--before-concurrency", type=int, default=2)
    parser.add_argument("--after-concurrency", type=int, default=2)
    parser.add_argument("--max-invalid-retries", type=int, default=1)
    parser.add_argument("--invalid-retry-delay-sec", type=float, default=60.0)
    parser.add_argument("--server-workspace-root", default="/bench-work")
    return parser.parse_args()


def _load_selection(path: Path) -> tuple[dict[str, Any], list[str], dict[str, str]]:
    value = json.loads(path.read_text(encoding="utf-8"))
    categories = value.get("categories") or {}
    tasks: list[str] = []
    task_category: dict[str, str] = {}
    for category, ids in categories.items():
        for task_id in ids:
            task_id = str(task_id)
            if task_id in task_category:
                raise ValueError(f"duplicate pilot task: {task_id}")
            tasks.append(task_id)
            task_category[task_id] = str(category)
    return value, tasks, task_category


def _atomic_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(
        json.dumps(value, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    temporary.replace(path)


def _read_event_metrics(path: str | None) -> dict[str, int]:
    metrics = {
        "tool_calls": 0,
        "tool_failures": 0,
        "provider_retries": 0,
        "context_compactions": 0,
    }
    if not path:
        return metrics
    event_path = Path(path)
    if not event_path.is_file():
        return metrics
    for line in event_path.read_text(encoding="utf-8", errors="replace").splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        event_type = str(event.get("type") or "")
        payload = event.get("payload") or {}
        if event_type in {"tool.call.completed", "browser.action.completed"}:
            metrics["tool_calls"] += 1
            success = payload.get("success")
            result = payload.get("result") or {}
            if success is False or result.get("isError") is True:
                metrics["tool_failures"] += 1
        elif event_type == "model.request.retried":
            metrics["provider_retries"] += 1
        elif event_type == "context.compaction.completed":
            metrics["context_compactions"] += 1
    return metrics


def _summarize_task_result(
    *, snapshot: str, category: str, attempt: int, result: Any
) -> dict[str, Any]:
    adapter_results = list(result.adapter_results or [result.adapter_result])
    metadata = adapter_results[-1].metadata if adapter_results else {}
    valid = bool(adapter_results) and all(
        item.metadata.get("valid") is True for item in adapter_results
    )
    usage = dict(result.usage_summary or {})
    cache_observations = list(metadata.get("cache_observations") or [])
    event_metrics = _read_event_metrics(metadata.get("eventsPath"))
    uncached = int(usage.get("input_tokens", 0) or 0)
    cached = int(usage.get("cache_read_tokens", 0) or 0)
    total_input = uncached + cached
    oracle = dict(result.oracle_result or {})
    scoring = dict(result.scoring or {})
    return {
        "snapshot": snapshot,
        "category": category,
        "task_id": result.task_id,
        "attempt": attempt,
        "valid": valid,
        "adapter_ok": bool(result.adapter_result.ok),
        "terminal_status": metadata.get("terminalStatus"),
        "failure_class": metadata.get("failureClass"),
        "elapsed_sec": result.elapsed_sec,
        "rounds": int(metadata.get("rounds", usage.get("request_count", 0)) or 0),
        "phases": int(metadata.get("phases", len(adapter_results)) or 0),
        "usage": {
            "uncached_input_tokens": uncached,
            "cached_input_tokens": cached,
            "total_input_tokens": total_input,
            "output_tokens": int(usage.get("output_tokens", 0) or 0),
            "cache_write_tokens": int(usage.get("cache_write_tokens", 0) or 0),
            "total_tokens": int(usage.get("total_tokens", 0) or 0),
            "cache_hit_rate": (cached / total_input) if total_input else None,
        },
        "oracle_outcome_score": oracle.get("outcome_score"),
        "oracle_checks": oracle.get("checks"),
        "combined_score": scoring.get("combined_score"),
        "process_score": scoring.get("process_score"),
        "security_score": scoring.get("security_score"),
        "cache_observations": cache_observations,
        **event_metrics,
        "sandbox": str(result.sandbox),
    }


def _aggregate(rows: list[dict[str, Any]]) -> dict[str, Any]:
    valid = [row for row in rows if row["valid"]]
    outcomes = [
        float(row["oracle_outcome_score"])
        for row in valid
        if isinstance(row.get("oracle_outcome_score"), (int, float))
    ]
    total_input = sum(row["usage"]["total_input_tokens"] for row in valid)
    cached = sum(row["usage"]["cached_input_tokens"] for row in valid)
    observations = [
        observation
        for row in valid
        for observation in row.get("cache_observations", [])
    ]
    eligible_transitions = sum(
        observation.get("state") not in {"initial", "not_eligible"}
        for observation in observations
    )
    broken_transitions = sum(
        observation.get("state") == "broken" for observation in observations
    )
    degraded_transitions = sum(
        observation.get("state") == "degraded" for observation in observations
    )
    task_cache_peaks = [
        max(
            float(observation.get("cache_hit_rate") or 0)
            for observation in row.get("cache_observations", [])
        )
        for row in valid
        if row.get("cache_observations")
    ]

    def cache_rate_for_rounds(minimum: int, maximum: int | None = None) -> float | None:
        selected = [
            observation
            for observation in observations
            if int(observation.get("round") or 0) >= minimum
            and (maximum is None or int(observation.get("round") or 0) <= maximum)
        ]
        selected_input = sum(int(item.get("input_tokens") or 0) for item in selected)
        selected_cached = sum(
            int(item.get("cached_input_tokens") or 0) for item in selected
        )
        return selected_cached / selected_input if selected_input else None

    tool_calls = sum(row["tool_calls"] for row in valid)
    tool_failures = sum(row["tool_failures"] for row in valid)
    return {
        "valid_tasks": len(valid),
        "invalid_tasks": len(rows) - len(valid),
        "scored_tasks": len(outcomes),
        "mean_oracle_outcome": statistics.fmean(outcomes) if outcomes else None,
        "full_success_rate": (
            sum(score >= 0.999 for score in outcomes) / len(outcomes)
            if outcomes
            else None
        ),
        "partial_success_rate": (
            sum(0 < score < 0.999 for score in outcomes) / len(outcomes)
            if outcomes
            else None
        ),
        "zero_outcome_rate": (
            sum(score <= 0 for score in outcomes) / len(outcomes)
            if outcomes
            else None
        ),
        "mean_rounds": (
            statistics.fmean(row["rounds"] for row in valid) if valid else None
        ),
        "total_rounds": sum(row["rounds"] for row in valid),
        "total_input_tokens": total_input,
        "cached_input_tokens": cached,
        "output_tokens": sum(row["usage"]["output_tokens"] for row in valid),
        "total_tokens": sum(row["usage"]["total_tokens"] for row in valid),
        "token_weighted_cache_hit_rate": cached / total_input if total_input else None,
        "mean_task_peak_cache_hit_rate": (
            statistics.fmean(task_cache_peaks) if task_cache_peaks else None
        ),
        "cache_hit_rate_by_phase_round": {
            "2_4": cache_rate_for_rounds(2, 4),
            "5_8": cache_rate_for_rounds(5, 8),
            "9_plus": cache_rate_for_rounds(9),
        },
        "cache_eligible_transitions": eligible_transitions,
        "cache_broken_transitions": broken_transitions,
        "cache_degraded_transitions": degraded_transitions,
        "cache_break_rate": (
            broken_transitions / eligible_transitions if eligible_transitions else None
        ),
        "cache_disruption_rate": (
            (broken_transitions + degraded_transitions) / eligible_transitions
            if eligible_transitions
            else None
        ),
        "tool_calls": tool_calls,
        "tool_failures": tool_failures,
        "tool_failure_rate": tool_failures / tool_calls if tool_calls else None,
        "provider_retries": sum(row["provider_retries"] for row in valid),
        "context_compactions": sum(row["context_compactions"] for row in valid),
    }


def _percent(value: Any) -> str:
    return "—" if value is None else f"{float(value) * 100:.2f}%"


def _number(value: Any, digits: int = 3) -> str:
    return "—" if value is None else f"{float(value):.{digits}f}"


def _percentage_point_delta(after: Any, before: Any) -> str:
    if after is None or before is None:
        return "—"
    return f"{(float(after) - float(before)) * 100:+.2f} pp"


def _write_report(path: Path, payload: dict[str, Any]) -> None:
    before = payload["aggregate"]["before"]
    after = payload["aggregate"]["after"]
    pairs = payload["pairs"]
    comparable = [pair for pair in pairs if pair["comparable"]]
    oracle_delta = after["mean_oracle_outcome"] - before["mean_oracle_outcome"]
    full_success_delta = after["full_success_rate"] - before["full_success_rate"]
    cache_delta = (
        after["token_weighted_cache_hit_rate"]
        - before["token_weighted_cache_hit_rate"]
    )
    round_delta = after["mean_rounds"] / before["mean_rounds"] - 1
    token_delta = after["total_tokens"] / before["total_tokens"] - 1
    break_rate_delta = after["cache_break_rate"] / before["cache_break_rate"] - 1
    disruption_delta = (
        after["cache_disruption_rate"] / before["cache_disruption_rate"] - 1
    )
    is_full_suite = payload["selected_tasks"] == 106
    report_title = (
        "OpenTopia Harness-Bench 全量配对报告"
        if is_full_suite
        else "OpenTopia Harness-Bench 第一阶段试点报告"
    )
    lines = [
        f"# {report_title}",
        "",
        f"- 评测集：Qihoo360/Harness-Bench，固定提交 `{payload['benchmark_commit']}`",
        f"- 试点规模：{payload['selected_tasks']} 道离线任务，Before/After 严格配对",
        f"- 模型配置：`{payload['model']}`，推理强度 `{payload['reasoning_effort']}`",
        f"- Artifact：Before `{payload['artifact_sha256']['before']}`；After `{payload['artifact_sha256']['after']}`",
        f"- 并发结构：每个快照 1 个常驻 Server；Before {payload['concurrency']['before']} 会话、After {payload['concurrency']['after']} 会话并发",
        "- 本阶段目的：验证 Oracle、共享 Server 多会话、Round/Token/缓存/工具日志链路；过程分 LLM 在试点阶段关闭，避免在链路校准前产生额外阅卷成本。",
        "",
        "## 汇总",
        "",
        "| 指标 | Before | After |",
        "|---|---:|---:|",
        f"| 有效任务 | {before['valid_tasks']} | {after['valid_tasks']} |",
        f"| Oracle 平均结果分 | {_number(before['mean_oracle_outcome'])} | {_number(after['mean_oracle_outcome'])} |",
        f"| 完全成功率 | {_percent(before['full_success_rate'])} | {_percent(after['full_success_rate'])} |",
        f"| 部分成功率 | {_percent(before['partial_success_rate'])} | {_percent(after['partial_success_rate'])} |",
        f"| 平均模型轮数 | {_number(before['mean_rounds'], 2)} | {_number(after['mean_rounds'], 2)} |",
        f"| Token 加权缓存命中率 | {_percent(before['token_weighted_cache_hit_rate'])} | {_percent(after['token_weighted_cache_hit_rate'])} |",
        f"| 缓存破坏 / 退化 | {before['cache_broken_transitions']} / {before['cache_degraded_transitions']} | {after['cache_broken_transitions']} / {after['cache_degraded_transitions']} |",
        f"| 工具调用 / 失败 | {before['tool_calls']} / {before['tool_failures']} | {after['tool_calls']} / {after['tool_failures']} |",
        f"| Provider 重试 | {before['provider_retries']} | {after['provider_retries']} |",
        f"| 总 Token | {before['total_tokens']} | {after['total_tokens']} |",
        "",
        "## 缓存与轮次",
        "",
        "| 指标 | Before | After | 变化 |",
        "|---|---:|---:|---:|",
        f"| Token 加权缓存命中率 | {_percent(before['token_weighted_cache_hit_rate'])} | {_percent(after['token_weighted_cache_hit_rate'])} | {cache_delta * 100:+.2f} pp |",
        f"| 单任务平均峰值命中率 | {_percent(before['mean_task_peak_cache_hit_rate'])} | {_percent(after['mean_task_peak_cache_hit_rate'])} | {_percentage_point_delta(after['mean_task_peak_cache_hit_rate'], before['mean_task_peak_cache_hit_rate'])} |",
        f"| 每阶段第 2–4 轮命中率 | {_percent(before['cache_hit_rate_by_phase_round']['2_4'])} | {_percent(after['cache_hit_rate_by_phase_round']['2_4'])} | {_percentage_point_delta(after['cache_hit_rate_by_phase_round']['2_4'], before['cache_hit_rate_by_phase_round']['2_4'])} |",
        f"| 每阶段第 5–8 轮命中率 | {_percent(before['cache_hit_rate_by_phase_round']['5_8'])} | {_percent(after['cache_hit_rate_by_phase_round']['5_8'])} | {_percentage_point_delta(after['cache_hit_rate_by_phase_round']['5_8'], before['cache_hit_rate_by_phase_round']['5_8'])} |",
        f"| 每阶段第 9+ 轮命中率 | {_percent(before['cache_hit_rate_by_phase_round']['9_plus'])} | {_percent(after['cache_hit_rate_by_phase_round']['9_plus'])} | {_percentage_point_delta(after['cache_hit_rate_by_phase_round']['9_plus'], before['cache_hit_rate_by_phase_round']['9_plus'])} |",
        f"| 缓存破坏率 | {_percent(before['cache_break_rate'])} | {_percent(after['cache_break_rate'])} | {break_rate_delta * 100:+.2f}% |",
        f"| 缓存破坏或退化率 | {_percent(before['cache_disruption_rate'])} | {_percent(after['cache_disruption_rate'])} | {disruption_delta * 100:+.2f}% |",
        "",
        "多阶段任务会在新阶段重新从第 1 轮计数，因此轮次分层按“阶段内轮次”统计。第 9+ 轮 Before 样本明显少于 After，只用于观察稳态趋势，不单独作为简历提升数字。",
        "",
        "## 试点结论",
        "",
        f"- After 的总体缓存命中率提升 {cache_delta * 100:.2f} 个百分点；缓存破坏事件由 {before['cache_broken_transitions']} 次降至 {after['cache_broken_transitions']} 次，且 After 的有效转移更多，破坏率相对下降 {-break_rate_delta * 100:.2f}%，破坏或退化率相对下降 {-disruption_delta * 100:.2f}%。",
        f"- After 平均轮数增加 {round_delta * 100:.2f}%，总 Token 增加 {token_delta * 100:.2f}%。因此本试点支持“缓存稳定性改善”，不支持“总成本下降”。",
        f"- Oracle 平均结果分变化 {oracle_delta * 100:+.2f} 个百分点，完全成功率变化 {full_success_delta * 100:+.2f} 个百分点，方向不一致；需在全量 106 对与复测后才能形成质量结论。",
        f"- 工具失败率从 {_percent(before['tool_failure_rate'])} 变为 {_percent(after['tool_failure_rate'])}。该项需结合具体失败类型和最终 Oracle 逐题审查，不能直接解释为工具能力下降。",
        "",
        "## 配对完整性",
        "",
        f"共有 {len(comparable)}/{len(pairs)} 对具备双方有效结果。基础设施、Provider 或适配器无效运行没有按 0 分计入。",
        "",
        "## 解释边界",
        "",
        (
            "本报告已覆盖全量 106 对 Oracle，但过程分/安全门控和不稳定或结论相反任务的复测尚未完成，因此仍不是冻结的最终简历数字。缓存比较必须同时报告轮次分层、稳态区间与缓存破坏率，不能只比较总命中率。"
            if is_full_suite
            else "本报告是第一阶段工程试点，不是最终简历数字。只有完成全量 106 对、补齐过程分/安全门控并对不稳定或结论相反的题复测后，才冻结最终指标。缓存比较必须同时报告轮次分层、稳态区间与缓存破坏率，不能只比较总命中率。"
        ),
        "",
    ]
    path.write_text("\n".join(lines), encoding="utf-8")


def main() -> int:
    args = _arguments()
    harness_root = args.harness_root.resolve()
    output_root = args.output_root.resolve()
    work_root = args.work_root.resolve()
    selection_path = args.selection.resolve()
    before_token = os.environ.get("OPENTOPIA_HB_BEFORE_TOKEN", "")
    after_token = os.environ.get("OPENTOPIA_HB_AFTER_TOKEN", "")
    if not before_token or not after_token:
        raise SystemExit("shared server API tokens are missing from the process environment")

    selection, selected_tasks, task_category = _load_selection(selection_path)
    if args.task_ids.strip():
        requested = [item.strip() for item in args.task_ids.split(",") if item.strip()]
        unknown = [item for item in requested if item not in task_category]
        if unknown:
            raise SystemExit(f"--task-ids contains tasks outside the pinned pilot: {unknown}")
        selected_tasks = requested
    actual_commit = subprocess.run(
        ["git", "-C", str(harness_root), "rev-parse", "HEAD"],
        text=True,
        capture_output=True,
        check=True,
    ).stdout.strip()
    expected_commit = str(selection.get("benchmarkCommit") or "")
    if actual_commit != expected_commit:
        raise SystemExit(
            f"Harness-Bench commit mismatch: expected {expected_commit}, got {actual_commit}"
        )

    harness_src = harness_root / "src"
    sys.path.insert(0, str(harness_src))
    from harnessbench.models import AppConfig
    from harnessbench.runner import run_task
    from harnessbench.tasks import load_tasks
    import harnessbench.runner as benchmark_runner

    from evaluation.integrations.harnessbench.opentopia_adapter import (
        OpenTopiaSharedServerAdapter,
    )

    benchmark_runner.build_adapter = lambda _name: OpenTopiaSharedServerAdapter()
    # Hooks for browser and local-API tasks normally open a public tunnel for
    # remote harnesses.  This target is local in Docker, so keep fixtures
    # private and let the adapter bridge loopback through host.docker.internal.
    os.environ.setdefault("HARNESSBENCH_PUBLIC_URL_TEMPLATE", "{local_url}")
    os.environ["HARNESSBENCH_SKIP_PROCESS_GRADE"] = "1"
    os.environ["HARNESSBENCH_SKIP_ORACLE_QUALITY_LLM"] = "1"

    all_tasks = load_tasks(harness_root / "tasks")
    missing = [task_id for task_id in selected_tasks if task_id not in all_tasks]
    if missing:
        raise SystemExit(f"pilot selection contains missing tasks: {missing}")

    adapter_script = Path(__file__).resolve().parents[2] / "adapters" / "opentopia-http.mjs"
    output_root.mkdir(parents=True, exist_ok=True)
    status_path = output_root / args.status_file
    artifact_sha256 = {
        "before": args.before_artifact_sha256.upper(),
        "after": args.after_artifact_sha256.upper(),
    }
    status_lock = threading.Lock()
    final_rows: dict[tuple[str, str], dict[str, Any]] = {}
    attempts: list[dict[str, Any]] = []
    if status_path.is_file():
        try:
            previous_status = json.loads(status_path.read_text(encoding="utf-8"))
            previous_attempts = [
                item
                for item in previous_status.get("attempts", [])
                if isinstance(item, dict)
            ]
            if (
                previous_attempts
                and previous_status.get("artifact_sha256") != artifact_sha256
            ):
                raise SystemExit(
                    "status file belongs to different Before/After artifacts; "
                    "use a fresh output root instead of mixing snapshots"
                )
            attempts = previous_attempts
            for item in attempts:
                key = (str(item.get("snapshot") or ""), str(item.get("task_id") or ""))
                if item.get("valid") is True:
                    final_rows[key] = item
        except json.JSONDecodeError:
            attempts = []
            final_rows = {}

    if args.reuse_before_status:
        reuse_status_path = args.reuse_before_status.resolve()
        reuse_status = json.loads(reuse_status_path.read_text(encoding="utf-8"))
        reuse_artifacts = reuse_status.get("artifact_sha256") or {}
        recorded_before_sha = str(reuse_artifacts.get("before") or "").upper()
        if not recorded_before_sha and args.reuse_before_provenance:
            provenance = json.loads(
                args.reuse_before_provenance.resolve().read_text(encoding="utf-8")
            )
            recorded_before_sha = str(provenance.get("binarySha256") or "").upper()
        if recorded_before_sha != artifact_sha256["before"]:
            raise SystemExit(
                "reused Before status belongs to a different Before artifact"
            )
        for item in reuse_status.get("attempts", []):
            if not isinstance(item, dict):
                continue
            task_id = str(item.get("task_id") or "")
            key = ("before", task_id)
            if (
                item.get("snapshot") == "before"
                and item.get("valid") is True
                and task_id in selected_tasks
                and key not in final_rows
            ):
                attempts.append(item)
                final_rows[key] = item

    variants = {
        "before": {
            "url": args.before_url,
            "token": before_token,
            "provider": args.before_provider,
            "concurrency": args.before_concurrency,
        },
        "after": {
            "url": args.after_url,
            "token": after_token,
            "provider": args.after_provider,
            "concurrency": args.after_concurrency,
        },
    }
    semaphores = {
        name: threading.Semaphore(int(cfg["concurrency"]))
        for name, cfg in variants.items()
    }
    circuit_lock = threading.Lock()
    circuit_events = {name: threading.Event() for name in variants}
    circuit_failure_times: dict[str, list[float]] = {name: [] for name in variants}
    circuit_state: dict[str, dict[str, Any]] = {
        name: {
            "open": False,
            "opened_at": None,
            "reason": None,
            "immediate_failures_in_window": 0,
        }
        for name in variants
    }

    def persist_status() -> None:
        with status_lock:
            _atomic_json(
                status_path,
                {
                    "updated_at": datetime.now(timezone.utc).isoformat(),
                    "selected_tasks": len(selected_tasks),
                    "valid_results": len(final_rows),
                    "artifact_sha256": artifact_sha256,
                    "circuit_breakers": circuit_state,
                    "attempts": attempts,
                },
            )

    def update_shared_provider_circuit(snapshot: str, row: dict[str, Any]) -> None:
        """Stop dispatch when one shared provider becomes globally unusable.

        OpenTopia deliberately blocks a provider after a permanent quota error.
        Once that happens, every new session fails before its first model round.
        A short-window circuit breaker prevents the queued benchmark tasks from
        being turned into a long sequence of meaningless invalid attempts.
        """

        usage = row.get("usage") if isinstance(row.get("usage"), dict) else {}
        immediate_provider_failure = (
            row.get("valid") is not True
            and row.get("adapter_ok") is False
            and int(row.get("rounds") or 0) == 0
            and int(usage.get("total_tokens") or 0) == 0
            and float(row.get("elapsed_sec") or 0.0) <= 10.0
        )
        if not immediate_provider_failure:
            return

        now = time.monotonic()
        threshold = max(2, int(variants[snapshot]["concurrency"]))
        with circuit_lock:
            recent = [
                observed_at
                for observed_at in circuit_failure_times[snapshot]
                if now - observed_at <= 30.0
            ]
            recent.append(now)
            circuit_failure_times[snapshot] = recent
            circuit_state[snapshot]["immediate_failures_in_window"] = len(recent)
            if len(recent) < threshold or circuit_events[snapshot].is_set():
                return
            circuit_state[snapshot].update(
                {
                    "open": True,
                    "opened_at": datetime.now(timezone.utc).isoformat(),
                    "reason": "shared_provider_immediate_failure_burst",
                }
            )
            circuit_events[snapshot].set()
            print(
                f"[harnessbench] {snapshot} shared-provider circuit opened "
                f"after {len(recent)} immediate failures",
                flush=True,
            )

    def execute(snapshot: str, task_id: str) -> dict[str, Any]:
        cfg = variants[snapshot]
        last_row: dict[str, Any] | None = None
        if circuit_events[snapshot].is_set():
            return {
                "snapshot": snapshot,
                "category": task_category[task_id],
                "task_id": task_id,
                "attempt": 0,
                "valid": False,
                "skipped": True,
                "failure_class": "shared_provider_circuit_open",
            }
        with semaphores[snapshot]:
            for attempt in range(1, args.max_invalid_retries + 2):
                if circuit_events[snapshot].is_set():
                    break
                app = AppConfig(
                    project_root=harness_root,
                    data_dir=output_root / snapshot / "data",
                    tasks_dir=harness_root / "tasks",
                    results_dir=output_root / snapshot / "results",
                    # Keep executable task paths short on Windows.  Several
                    # official Oracles launch subprocesses with a task-local
                    # cwd and Win32 CreateProcess still rejects paths beyond
                    # the legacy limit even when Python can enumerate them.
                    work_root=work_root / snapshot,
                    default_timeout_sec=600,
                    default_rounds=1,
                )
                for directory in (app.data_dir, app.results_dir, app.work_root):
                    directory.mkdir(parents=True, exist_ok=True)
                model_cfg = {
                    "adapter": "opentopia_shared_server",
                    "session_prefix": f"harnessbench-{snapshot}",
                    "model": args.model,
                    "reasoning_effort": args.reasoning_effort,
                    "server_base_url": cfg["url"],
                    "api_token": cfg["token"],
                    "provider_id": cfg["provider"],
                    "workspace_host_root": str(work_root),
                    "workspace_server_root": args.server_workspace_root,
                    "adapter_script": str(adapter_script),
                    "title_prefix": f"HarnessBench {snapshot}",
                }
                try:
                    result = run_task(
                        app,
                        all_tasks[task_id],
                        f"opentopia-{snapshot}",
                        model_cfg,
                        "live",
                        keep_workspace=True,
                    )
                    row = _summarize_task_result(
                        snapshot=snapshot,
                        category=task_category[task_id],
                        attempt=attempt,
                        result=result,
                    )
                except Exception as exc:
                    row = {
                        "snapshot": snapshot,
                        "category": task_category[task_id],
                        "task_id": task_id,
                        "attempt": attempt,
                        "valid": False,
                        "failure_class": "runner_exception",
                        "error_type": type(exc).__name__,
                        "error": str(exc),
                        "traceback": traceback.format_exc(),
                    }
                with status_lock:
                    attempts.append(row)
                update_shared_provider_circuit(snapshot, row)
                persist_status()
                last_row = row
                if row.get("valid"):
                    return row
                if attempt <= args.max_invalid_retries:
                    delay = max(0.0, float(args.invalid_retry_delay_sec))
                    if delay:
                        print(
                            f"[harnessbench] {snapshot} {task_id} invalid "
                            f"attempt={attempt}; cooldown={delay:.0f}s",
                            flush=True,
                        )
                        time.sleep(delay)
        return last_row or {
            "snapshot": snapshot,
            "category": task_category[task_id],
            "task_id": task_id,
            "attempt": 0,
            "valid": False,
            "skipped": True,
            "failure_class": "shared_provider_circuit_open",
        }

    started = time.perf_counter()
    futures: dict[Future[dict[str, Any]], tuple[str, str]] = {}
    # Keep an independent worker pool for each shared server.  A single mixed
    # pool can be starved by submission order (for example, every Before job
    # occupying or waiting on the first slots while the After server is idle).
    # Separate pools guarantee that each server continuously uses its own
    # configured multi-session concurrency.
    executors = {
        snapshot: ThreadPoolExecutor(
            max_workers=int(cfg["concurrency"]),
            thread_name_prefix=f"harnessbench-{snapshot}",
        )
        for snapshot, cfg in variants.items()
    }
    try:
        for snapshot in ("before", "after"):
            for task_id in selected_tasks:
                if (snapshot, task_id) in final_rows:
                    continue
                future = executors[snapshot].submit(execute, snapshot, task_id)
                futures[future] = (snapshot, task_id)
        for future in as_completed(futures):
            snapshot, task_id = futures[future]
            row = future.result()
            if row.get("valid"):
                final_rows[(snapshot, task_id)] = row
            persist_status()
            print(
                f"[harnessbench] {snapshot} {task_id} valid={row.get('valid')} "
                f"attempt={row.get('attempt')}",
                flush=True,
            )
    finally:
        for executor in executors.values():
            executor.shutdown(wait=True, cancel_futures=False)

    rows = []
    for snapshot in ("before", "after"):
        for task_id in selected_tasks:
            row = final_rows.get((snapshot, task_id))
            if row is None:
                row = next(
                    (
                        item
                        for item in reversed(attempts)
                        if item["snapshot"] == snapshot and item["task_id"] == task_id
                    ),
                    {
                        "snapshot": snapshot,
                        "category": task_category[task_id],
                        "task_id": task_id,
                        "attempt": 0,
                        "valid": False,
                        "skipped": True,
                        "failure_class": "not_run_after_circuit_breaker",
                    },
                )
            rows.append(row)
    pairs = []
    for task_id in selected_tasks:
        before = final_rows.get(("before", task_id))
        after = final_rows.get(("after", task_id))
        comparable = before is not None and after is not None
        pairs.append(
            {
                "task_id": task_id,
                "category": task_category[task_id],
                "comparable": comparable,
                "before_outcome": before.get("oracle_outcome_score") if before else None,
                "after_outcome": after.get("oracle_outcome_score") if after else None,
                "outcome_delta": (
                    float(after["oracle_outcome_score"])
                    - float(before["oracle_outcome_score"])
                    if comparable
                    and isinstance(before.get("oracle_outcome_score"), (int, float))
                    and isinstance(after.get("oracle_outcome_score"), (int, float))
                    else None
                ),
            }
        )

    payload = {
        "schema_version": 1,
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "benchmark": "Qihoo360/Harness-Bench",
        "benchmark_commit": actual_commit,
        "selection": str(selection_path),
        "selected_tasks": len(selected_tasks),
        "model": args.model,
        "reasoning_effort": args.reasoning_effort,
        "artifact_sha256": artifact_sha256,
        "concurrency": {
            "before": args.before_concurrency,
            "after": args.after_concurrency,
        },
        "wall_elapsed_sec": round(time.perf_counter() - started, 3),
        "process_grade_enabled": False,
        "rows": rows,
        "attempts": attempts,
        "pairs": pairs,
        "aggregate": {
            "before": _aggregate([row for row in rows if row["snapshot"] == "before"]),
            "after": _aggregate([row for row in rows if row["snapshot"] == "after"]),
        },
    }
    _atomic_json(output_root / args.summary_file, payload)
    _write_report(output_root / args.report_file, payload)
    persist_status()
    comparable = sum(pair["comparable"] for pair in pairs)
    print(json.dumps({"output_root": str(output_root), "valid_pairs": comparable}, ensure_ascii=False))
    return 0 if comparable == len(selected_tasks) else 2


if __name__ == "__main__":
    raise SystemExit(main())
