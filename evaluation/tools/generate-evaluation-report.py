#!/usr/bin/env python3
"""Generate an auditable Chinese before/after report from paired summaries.

The input summary files are produced by the benchmark-specific summarizers.
This program reads aggregate metrics only; it never opens task instructions,
model messages, patches, tool arguments, or tool output.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


def read_summary(path: Path) -> dict[str, Any]:
    data = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(data, dict) or not isinstance(data.get("before"), dict) or not isinstance(data.get("after"), dict):
        raise ValueError(f"Invalid paired summary: {path}")
    return data


def number(value: Any) -> float | None:
    return float(value) if isinstance(value, (int, float)) and not isinstance(value, bool) else None


def display(value: Any, *, percent: bool = False) -> str:
    value = number(value)
    if value is None:
        return "—"
    if percent:
        return f"{value * 100:.1f}%"
    if value.is_integer():
        return f"{int(value):,}"
    return f"{value:,.2f}"


def relative(before: Any, after: Any) -> float | None:
    before_value, after_value = number(before), number(after)
    if before_value is None or after_value is None or before_value == 0:
        return None
    return (after_value - before_value) / before_value


def percentage_points(before: Any, after: Any) -> float | None:
    before_value, after_value = number(before), number(after)
    if before_value is None or after_value is None:
        return None
    return (after_value - before_value) * 100


def signed_percent(value: float | None) -> str:
    return "—" if value is None else f"{value * 100:+.1f}%"


def signed_points(value: float | None) -> str:
    return "—" if value is None else f"{value:+.1f} 个百分点"


def metric_rows(summary: dict[str, Any], correctness_key: str) -> list[tuple[str, str, bool, str]]:
    return [
        ("正确性", correctness_key, True, "任务成功/官方 resolved"),
        ("平均 agent 时长", "agentDurationMeanSeconds", False, "秒"),
        ("中位 agent 时长", "agentDurationMedianSeconds", False, "秒"),
        ("平均输入 token", "inputTokensMean", False, "token"),
        ("平均缓存 token", "cacheTokensMean", False, "token"),
        ("缓存命中率", "cacheHitRate", True, "缓存 token / 输入 token"),
        ("平均未缓存输入 token", "uncachedInputTokensMean", False, "token"),
        ("平均输出 token", "outputTokensMean", False, "token"),
        ("平均推理 token", "reasoningTokensMean", False, "token"),
        ("平均模型请求数", "modelRequestsMean", False, "次"),
        ("平均 Provider 响应数", "providerResponsesMean", False, "次"),
        ("平均开始工具调用数", "toolCallsStartedMean", False, "次"),
        ("平均完成工具调用数", "toolCallsFinishedMean", False, "次"),
        ("受控超时数", "controlledTimeouts", False, "个"),
    ]


def table(title: str, summary: dict[str, Any], correctness_key: str) -> list[str]:
    before, after = summary["before"], summary["after"]
    lines = [f"## {title}", "", "| 指标 | Before | After | After 相对 Before |", "|---|---:|---:|---:|"]
    for label, key, is_percent, _ in metric_rows(summary, correctness_key):
        delta = relative(before.get(key), after.get(key))
        if is_percent:
            delta_display = signed_points(percentage_points(before.get(key), after.get(key)))
        else:
            delta_display = signed_percent(delta)
        lines.append(
            f"| {label} | {display(before.get(key), percent=is_percent)} | "
            f"{display(after.get(key), percent=is_percent)} | {delta_display} |"
        )
    return lines


def improvement_claims(summary: dict[str, Any], correctness_key: str, correctness_label: str) -> list[str]:
    before, after = summary["before"], summary["after"]
    claims: list[str] = []
    correctness_delta = percentage_points(before.get(correctness_key), after.get(correctness_key))
    # A zero delta is a measured tie, not an improvement claim.  In
    # particular, a 0% -> 0% result must never be presented as evidence of
    # better agent capability in a résumé-oriented summary.
    if correctness_delta is not None and correctness_delta != 0:
        direction = "提高" if correctness_delta >= 0 else "下降"
        claims.append(f"{correctness_label}{direction} {abs(correctness_delta):.1f} 个百分点")
    for key, label in (
        ("uncachedInputTokensMean", "平均未缓存输入 token"),
        ("inputTokensMean", "平均输入 token"),
        ("agentDurationMeanSeconds", "平均 agent 时长"),
        ("modelRequestsMean", "平均模型请求数"),
    ):
        delta = relative(before.get(key), after.get(key))
        if delta is None:
            continue
        direction = "降低" if delta <= 0 else "增加"
        claims.append(f"{label}{direction} {abs(delta) * 100:.1f}%")
    cache_delta = percentage_points(before.get("cacheHitRate"), after.get("cacheHitRate"))
    if cache_delta is not None:
        direction = "提高" if cache_delta >= 0 else "下降"
        claims.append(f"缓存命中率{direction} {abs(cache_delta):.1f} 个百分点")
    return claims


def unchanged_correctness_note(
    terminal: dict[str, Any], swe: dict[str, Any]
) -> str | None:
    """Return a conservative capability caveat when neither benchmark improved."""
    terminal_before = number(terminal["before"].get("taskSuccessRate"))
    terminal_after = number(terminal["after"].get("taskSuccessRate"))
    swe_before = number(swe["before"].get("resolveRate"))
    swe_after = number(swe["after"].get("resolveRate"))
    if terminal_before == terminal_after and swe_before == swe_after:
        return (
            "官方正确性未发生变化：Terminal-Bench 为 "
            f"{display(terminal_before, percent=True)} → {display(terminal_after, percent=True)}，"
            "SWE-bench Verified 为 "
            f"{display(swe_before, percent=True)} → {display(swe_after, percent=True)}；"
            "本轮只能证明效率指标的变化，不能据此声称 agent 能力或任务成功率提升。"
        )
    return None


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--terminal-summary", required=True, type=Path)
    parser.add_argument("--swe-summary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()

    terminal = read_summary(args.terminal_summary)
    swe = read_summary(args.swe_summary)
    terminal_count = len(terminal.get("pairedTasks", []))
    swe_count = len(swe.get("pairedInstances", []))

    lines = [
        "# OpenTopia before/after 公共基准评测报告",
        "",
        "## 可比性与边界",
        "",
        "- Before 是精确提交 `6d52d5e` 加仅 provider wire-schema 兼容层；After 是当前 schema-fix 工作树产物。",
        "- 两侧使用同一模型 `gpt-5.6-terra`、`high` 推理强度、相同任务镜像/权限策略与每响应 8,192 输出 token 上限。",
        "- 仅统计同一任务在两个快照都具有有效结果的配对；基础设施失败不计入能力指标，受控超时计入任务失败。",
        "- Terminal-Bench 与 SWE-bench 的成功定义不同，以下分别报告，不能混为完整基准或排行榜分数。",
        "",
        "## 覆盖度",
        "",
        f"- Terminal-Bench 配对任务：{terminal_count}",
        f"- SWE-bench Verified 配对实例：{swe_count}",
        "",
    ]
    lines.extend(table("Terminal-Bench", terminal, "taskSuccessRate"))
    lines.extend([""])
    lines.extend(table("SWE-bench Verified", swe, "resolveRate"))
    unchanged_note = unchanged_correctness_note(terminal, swe)
    if unchanged_note:
        lines.extend(["", "## 正确性解读限制", "", f"- {unchanged_note}"])
    lines.extend(["", "## 可用于简历的量化表述（需保留基准与样本量）", ""])
    terminal_claims = "；".join(improvement_claims(terminal, "taskSuccessRate", "Terminal-Bench 任务成功率"))
    swe_claims = "；".join(improvement_claims(swe, "resolveRate", "SWE-bench Verified resolved 率"))
    if terminal_claims:
        lines.append(f"- 在 {terminal_count} 个配对 Terminal-Bench 任务上，{terminal_claims}。")
    if swe_claims:
        lines.append(f"- 在 {swe_count} 个配对 SWE-bench Verified 实例上，{swe_claims}。")
    lines.extend(["", "完整方法、快照兼容性边界与失败处理见 `METHOD-20260827-SCHEMAFIX.md`。", ""])

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text("\n".join(lines), encoding="utf-8")


if __name__ == "__main__":
    main()
