"""Convert OpenTopia product events into Harness-Bench's proxy trace format.

Harness-Bench normally observes the model wire protocol through a per-task
HTTP proxy.  A shared OpenTopia server cannot point one global provider at
many short-lived proxies safely.  OpenTopia already records the same usage,
round, model-output, and tool-call information per thread, so this module
materializes an equivalent, task-local trace from those product events.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Iterable


def _as_int(value: Any) -> int:
    try:
        return int(value or 0)
    except (TypeError, ValueError):
        return 0


def _json_arguments(value: Any) -> str:
    if isinstance(value, str):
        return value
    return json.dumps(value if value is not None else {}, ensure_ascii=False)


def read_jsonl(path: Path) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    if not path.is_file():
        return rows
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        if not line.strip():
            continue
        try:
            row = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(row, dict):
            rows.append(row)
    return rows


@dataclass
class _Round:
    request_id: str
    round_number: int
    phase: int
    events: list[dict[str, Any]] = field(default_factory=list)
    usage: dict[str, Any] = field(default_factory=dict)
    context: dict[str, Any] = field(default_factory=dict)


def _group_rounds(events: Iterable[dict[str, Any]]) -> list[_Round]:
    rounds: list[_Round] = []
    by_request: dict[str, _Round] = {}
    pending_context: dict[str, dict[str, Any]] = {}
    current: _Round | None = None
    phase = 0

    for event in events:
        event_type = str(event.get("type") or "")
        payload = event.get("payload")
        if not isinstance(payload, dict):
            payload = {}

        if event_type in {
            "application.thread.created",
            "application.thread.reused",
        }:
            phase += 1
            current = None
            continue

        if event_type == "opentopia.model_context_built":
            request_id = str(payload.get("request_id") or "")
            if request_id:
                pending_context[request_id] = dict(payload)
            continue

        if event_type == "model.request.started":
            request_id = str(payload.get("requestId") or "")
            if not request_id:
                continue
            current = by_request.get(request_id)
            if current is None:
                current = _Round(
                    request_id=request_id,
                    round_number=_as_int(payload.get("round")),
                    phase=max(phase, 1),
                    context=pending_context.pop(request_id, {}),
                )
                rounds.append(current)
                by_request[request_id] = current
            current.events.append(event)
            continue

        if event_type == "model.usage":
            request_id = str(payload.get("requestId") or "")
            target = by_request.get(request_id) if request_id else current
            if target is not None:
                target.usage = dict(payload)
                target.events.append(event)
                current = target
            continue

        if current is not None:
            current.events.append(event)

    return [item for item in rounds if item.usage]


def _round_response(item: _Round) -> tuple[str, list[dict[str, Any]], list[dict[str, Any]]]:
    assistant_parts: list[str] = []
    calls: list[dict[str, Any]] = []
    results: list[dict[str, Any]] = []

    for event in item.events:
        event_type = str(event.get("type") or "")
        payload = event.get("payload")
        if not isinstance(payload, dict):
            continue
        if event_type == "opentopia.model_delta":
            text = payload.get("text")
            if isinstance(text, str):
                assistant_parts.append(text)
        elif event_type == "opentopia.assistant_message":
            text = payload.get("content") or payload.get("text")
            if isinstance(text, str) and text not in assistant_parts:
                assistant_parts.append(text)
        elif event_type == "tool.call.started":
            call = payload.get("call")
            if not isinstance(call, dict):
                call = {}
            call_id = str(call.get("id") or f"round-{item.round_number}-call-{len(calls) + 1}")
            calls.append(
                {
                    "id": call_id,
                    "type": "function",
                    "function": {
                        "name": str(call.get("name") or payload.get("name") or "unknown"),
                        "arguments": _json_arguments(call.get("input")),
                    },
                }
            )
        elif event_type in {"tool.call.completed", "browser.action.completed"}:
            result = payload.get("result")
            if not isinstance(result, dict):
                result = {"output": result}
            name = str(payload.get("name") or payload.get("action") or "unknown")
            call_id = ""
            metadata = result.get("metadata")
            if isinstance(metadata, dict):
                call_id = str(metadata.get("providerToolCallId") or metadata.get("callId") or "")
                name = str(metadata.get("toolName") or name)
            if not call_id and len(results) < len(calls):
                call_id = str(calls[len(results)].get("id") or "")
            output = result.get("output")
            if output is None:
                output = result.get("content")
            results.append(
                {
                    "role": "tool",
                    "tool_call_id": call_id,
                    "name": name,
                    "content": output if isinstance(output, str) else json.dumps(output, ensure_ascii=False),
                }
            )

    return "".join(assistant_parts), calls, results


def _cache_observation(item: _Round, previous: _Round | None) -> dict[str, Any]:
    usage = item.usage
    cached = _as_int(usage.get("cachedInputTokens"))
    total_input = _as_int(usage.get("inputTokens"))
    stable = str(item.context.get("stable_prefix_hash") or "")
    dynamic = str(item.context.get("dynamic_tail_hash") or "")
    observation: dict[str, Any] = {
        "round": item.round_number,
        "input_tokens": total_input,
        "cached_input_tokens": cached,
        "cache_hit_rate": (cached / total_input) if total_input > 0 else None,
        "stable_prefix_hash": stable,
        "dynamic_tail_hash": dynamic,
        "state": "initial",
        "reason": "first_observation",
    }
    if previous is None:
        return observation

    previous_cached = _as_int(previous.usage.get("cachedInputTokens"))
    previous_stable = str(previous.context.get("stable_prefix_hash") or "")
    previous_dynamic = str(previous.context.get("dynamic_tail_hash") or "")
    observation["previous_cached_input_tokens"] = previous_cached
    observation["stable_prefix_changed"] = bool(stable and previous_stable and stable != previous_stable)
    observation["dynamic_tail_changed"] = bool(dynamic and previous_dynamic and dynamic != previous_dynamic)
    if previous_cached <= 0:
        observation["state"] = "not_eligible"
        observation["reason"] = "previous_request_had_no_cache_read"
    elif cached == 0:
        observation["state"] = "broken"
        observation["reason"] = (
            "stable_prefix_changed"
            if observation["stable_prefix_changed"]
            else "provider_cache_miss_with_stable_prefix"
        )
    elif cached < previous_cached * 0.5:
        observation["state"] = "degraded"
        observation["reason"] = (
            "stable_prefix_changed"
            if observation["stable_prefix_changed"]
            else "cached_prefix_shrank"
        )
    else:
        observation["state"] = "reused"
        observation["reason"] = "cache_read_preserved"
    return observation


def materialize_harnessbench_trace(
    *,
    events_path: Path,
    proxy_dir: Path,
    prompts: list[str],
    task_id: str,
    session_id: str,
    model_id: str,
) -> dict[str, Any]:
    """Rebuild a deterministic Harness-Bench trace for one OpenTopia thread."""
    events = read_jsonl(events_path)
    rounds = _group_rounds(events)
    responses_dir = proxy_dir / "responses"
    responses_dir.mkdir(parents=True, exist_ok=True)
    for old in responses_dir.glob("opentopia-*.json"):
        old.unlink()

    conversation: list[dict[str, Any]] = []
    active_phase = 0
    usage_rows: list[str] = []
    observations: list[dict[str, Any]] = []
    previous: _Round | None = None

    for index, item in enumerate(rounds, start=1):
        while active_phase < item.phase:
            prompt_index = active_phase
            prompt = prompts[prompt_index] if prompt_index < len(prompts) else ""
            conversation.append({"role": "user", "content": prompt})
            active_phase += 1

        assistant_text, tool_calls, tool_results = _round_response(item)
        response_file = responses_dir / f"opentopia-{index:04d}.json"
        raw_record = {
            "task_id": task_id,
            "session_id": session_id,
            "model_id": model_id,
            "framework": "opentopia",
            "provider": "opentopia-product-events",
            "request_body": json.dumps({"messages": conversation}, ensure_ascii=False),
            "response_json": {
                "model": model_id,
                "choices": [
                    {
                        "message": {
                            "role": "assistant",
                            "content": assistant_text,
                            "tool_calls": tool_calls,
                        }
                    }
                ],
            },
            "opentopia": {
                "request_id": item.request_id,
                "round": item.round_number,
                "phase": item.phase,
                "context_manifest": item.context,
            },
        }
        response_file.write_text(
            json.dumps(raw_record, ensure_ascii=False, indent=2),
            encoding="utf-8",
        )

        total_input = _as_int(item.usage.get("inputTokens"))
        cached = _as_int(item.usage.get("cachedInputTokens"))
        output = _as_int(item.usage.get("outputTokens"))
        total = _as_int(item.usage.get("totalTokens")) or total_input + output
        usage_row = {
            "task_id": task_id,
            "session_id": session_id,
            "model_id": model_id,
            "framework": "opentopia",
            "provider": "opentopia-product-events",
            "raw_response_file": str(response_file),
            "input_tokens": max(0, total_input - cached),
            "output_tokens": output,
            "cache_read_tokens": cached,
            "cache_write_tokens": _as_int(item.usage.get("cacheWriteTokens")),
            "total_tokens": total,
            "reasoning_tokens": _as_int(item.usage.get("reasoningTokens")),
            "response_model": model_id,
            "opentopia_round": item.round_number,
            "opentopia_phase": item.phase,
        }
        usage_rows.append(json.dumps(usage_row, ensure_ascii=False))
        observations.append(_cache_observation(item, previous))
        previous = item

        assistant_message: dict[str, Any] = {
            "role": "assistant",
            "content": assistant_text,
        }
        if tool_calls:
            assistant_message["tool_calls"] = tool_calls
        conversation.append(assistant_message)
        conversation.extend(tool_results)

    requests_path = proxy_dir / "requests.jsonl"
    requests_path.write_text(
        ("\n".join(usage_rows) + "\n") if usage_rows else "",
        encoding="utf-8",
    )
    diagnostics_path = proxy_dir / "opentopia-cache-diagnostics.json"
    diagnostics_path.write_text(
        json.dumps({"observations": observations}, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )

    return {
        "rounds": len(rounds),
        "phases": active_phase,
        "usage_rows": len(usage_rows),
        "cache_observations": observations,
        "diagnostics_path": str(diagnostics_path),
    }
