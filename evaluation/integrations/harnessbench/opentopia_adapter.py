"""Harness-Bench adapter for a long-lived, multi-session OpenTopia server."""

from __future__ import annotations

import json
import os
import re
import subprocess
import threading
from pathlib import Path
from typing import Any

from harnessbench.adapters.base import BaseAdapter
from harnessbench.models import AdapterRunContext, AdapterRunResult

from .trace import materialize_harnessbench_trace, read_jsonl


_PROVIDER_FAILURE_MARKERS = (
    "provider request stalled",
    "provider request failed",
    "upstream",
    "rate limit",
    "quota",
    "overloaded",
    "connection refused",
    "connection reset",
)

_LOOPBACK_URL_RE = re.compile(
    r"(?P<scheme>https?://)(?:127\.0\.0\.1|localhost)(?P<port>:\d+)",
    re.IGNORECASE,
)


def _rewrite_container_loopback_urls(prompt: str) -> str:
    """Map host-side Harness-Bench fixtures into the target container."""

    return _LOOPBACK_URL_RE.sub(
        lambda match: (
            f"{match.group('scheme')}host.docker.internal{match.group('port')}"
        ),
        prompt,
    )


def _map_workspace(path: Path, host_root: Path, server_root: str) -> str:
    def normalized(value: Path) -> str:
        text = os.path.normcase(os.path.abspath(os.fspath(value.resolve())))
        # pathlib may surface Win32's extended-length spelling after fixture
        # hooks touch a long workspace.  Docker bind mounts use the ordinary
        # drive spelling; both names identify the same path.
        if text.startswith("\\\\?\\UNC\\"):
            text = "\\\\" + text[8:]
        elif text.startswith("\\\\?\\"):
            text = text[4:]
        return os.path.normpath(text)

    resolved = normalized(path)
    root = normalized(host_root)
    try:
        common = os.path.commonpath((resolved, root))
    except ValueError as exc:
        raise ValueError(f"workspace {resolved} is outside mounted root {root}") from exc
    if os.path.normcase(common) != os.path.normcase(root):
        raise ValueError(f"workspace {resolved} is outside mounted root {root}")
    relative = os.path.relpath(resolved, root)
    suffix = Path(relative).as_posix()
    return f"{server_root.rstrip('/')}/{suffix}" if suffix else server_root.rstrip("/")


def _terminal_status(events: list[dict[str, Any]]) -> tuple[str, str]:
    status = ""
    error = ""
    for event in events:
        event_type = str(event.get("type") or "")
        payload = event.get("payload")
        if not isinstance(payload, dict):
            payload = {}
        if event_type in {
            "application.turn.completed",
            "application.turn.awaiting_user_action",
        }:
            status = str(payload.get("status") or "")
            raw_error = payload.get("error")
            if isinstance(raw_error, str):
                error = raw_error
        elif event_type == "application.adapter_error":
            error = str(payload.get("message") or error)
    return status, error


class OpenTopiaSharedServerAdapter(BaseAdapter):
    """Run independent Harness-Bench sessions against one shared server."""

    name = "opentopia_shared_server"

    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._prompts: list[str] = []

    def run(self, ctx: AdapterRunContext) -> AdapterRunResult:
        cfg = ctx.model_config
        server_url = str(cfg.get("server_base_url") or "").rstrip("/")
        api_token = str(cfg.get("api_token") or "")
        provider_id = str(cfg.get("provider_id") or "default")
        model = str(cfg.get("model") or "")
        reasoning = str(cfg.get("reasoning_effort") or "high")
        host_root = Path(str(cfg.get("workspace_host_root") or "")).resolve()
        server_root = str(cfg.get("workspace_server_root") or "/bench-output")
        adapter_script = Path(str(cfg.get("adapter_script") or "")).resolve()
        node = str(cfg.get("node") or "node")
        if not server_url or not api_token or not model:
            return AdapterRunResult(ok=False, stderr="shared OpenTopia server configuration is incomplete")
        if not adapter_script.is_file():
            return AdapterRunResult(ok=False, stderr=f"OpenTopia HTTP adapter not found: {adapter_script}")

        bridged_prompt = _rewrite_container_loopback_urls(ctx.prompt)
        with self._lock:
            self._prompts.append(bridged_prompt)
            phase_index = len(self._prompts)
            prompts = list(self._prompts)

        events_dir = ctx.sandbox / "opentopia"
        events_dir.mkdir(parents=True, exist_ok=True)
        events_path = events_dir / "events.jsonl"
        state_path = events_dir / "target-state.json"
        prompt_path = events_dir / f"prompt-phase-{phase_index}.txt"
        prompt_path.write_text(bridged_prompt, encoding="utf-8")
        server_workspace = _map_workspace(ctx.workspace, host_root, server_root)
        phase_count = len(ctx.task.prompt_files or []) or 1

        env = os.environ.copy()
        env.update(
            {
                "OPENTOPIA_EVAL_BASE_URL": server_url,
                "OPENTOPIA_API_TOKEN": api_token,
                "OPENTOPIA_EVAL_PROVIDER_ID": provider_id,
                "OPENTOPIA_EVAL_MODEL_ID": model,
                "OPENTOPIA_EVAL_REASONING_EFFORT": reasoning,
                "OPENTOPIA_EVAL_APPROVAL_MODE": "approve",
                "OPENTOPIA_EVAL_TIMEOUT_MS": str(int(ctx.timeout_sec) * 1000),
                "OPENTOPIA_EVAL_POLL_MS": str(int(cfg.get("poll_ms", 500))),
                "OPENTOPIA_EVAL_COMPACT_EVENTS": "0",
                "OPENTOPIA_EVAL_TITLE_PREFIX": str(cfg.get("title_prefix") or "HarnessBench"),
                "AGENT_EVAL_WORKSPACE": server_workspace,
                "AGENT_EVAL_EVENTS_PATH": str(events_path),
                "AGENT_EVAL_PROMPT_FILE": str(prompt_path),
                "AGENT_EVAL_TASK_ID": ctx.task.task_id,
                "AGENT_EVAL_PHASE_ID": f"round-{phase_index}",
                "AGENT_EVAL_PHASE_INDEX": str(phase_index),
                "AGENT_EVAL_PHASE_COUNT": str(phase_count),
                "AGENT_EVAL_TARGET_STATE_PATH": str(state_path),
            }
        )
        command = [node, str(adapter_script)]
        try:
            completed = subprocess.run(
                command,
                # The product executes tools in ``server_workspace``.  The
                # host-side Node bridge only performs HTTP control-plane I/O,
                # so giving CreateProcess the long task sandbox as its cwd is
                # unnecessary and crosses the legacy Windows path limit for
                # descriptive Harness-Bench task ids.
                cwd=str(adapter_script.parent),
                text=True,
                capture_output=True,
                timeout=ctx.timeout_sec + 30,
                env=env,
                check=False,
            )
        except subprocess.TimeoutExpired as exc:
            return AdapterRunResult(
                ok=False,
                command=command,
                stdout=exc.stdout or "",
                stderr="OpenTopia adapter process exceeded the controlled timeout",
                metadata={"valid": False, "failureClass": "adapter_timeout"},
            )

        proxy_routes = Path(ctx.env["HARNESSBENCH_LLM_PROXY_ROUTES"])
        trace_metadata = materialize_harnessbench_trace(
            events_path=events_path,
            proxy_dir=proxy_routes.parent,
            prompts=prompts,
            task_id=ctx.task.task_id,
            session_id=ctx.session_id,
            model_id=model,
        )
        events = read_jsonl(events_path)
        status, product_error = _terminal_status(events)
        combined_error = " ".join(
            item for item in (completed.stderr, product_error) if item
        ).strip()
        lower_error = combined_error.lower()
        infrastructure_failure = completed.returncode != 0 or any(
            marker in lower_error for marker in _PROVIDER_FAILURE_MARKERS
        )
        valid = not infrastructure_failure and trace_metadata["usage_rows"] > 0
        ok = completed.returncode == 0 and status in {
            "succeeded",
            "waiting_user_action",
        }
        if not valid:
            ok = False

        state: dict[str, Any] = {}
        if state_path.is_file():
            try:
                state = json.loads(state_path.read_text(encoding="utf-8"))
            except json.JSONDecodeError:
                state = {}
        metadata = {
            "returncode": completed.returncode,
            "valid": valid,
            "failureClass": "provider_or_infrastructure" if infrastructure_failure else None,
            "terminalStatus": status,
            "threadId": state.get("threadId"),
            "eventsPath": str(events_path),
            "targetStatePath": str(state_path),
            "serverWorkspace": server_workspace,
            **trace_metadata,
        }
        return AdapterRunResult(
            ok=ok,
            command=command,
            stdout=json.dumps(metadata, ensure_ascii=False),
            stderr=combined_error,
            metadata=metadata,
        )
