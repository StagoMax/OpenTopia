"""Small, content-agnostic bridge for running the local Codex CLI in evals.

The benchmark task remains in its official Linux container.  Codex runs from
an otherwise empty controller directory on the host and receives the exact
container id plus an instruction to use ``docker exec`` only for task work.
This lets the official verifier inspect the same container that the agent
modified while preserving Codex's native ChatGPT-account authentication.
"""

from __future__ import annotations

import json
import os
import shutil
import signal
import subprocess
import tempfile
import time
from hashlib import sha256
from dataclasses import dataclass, replace
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


@dataclass(frozen=True)
class CodexRunResult:
    """Content-free telemetry retained for a single ``codex exec`` run."""

    return_code: int | None
    timed_out: bool
    turn_succeeded: bool
    usage: dict[str, int]
    telemetry: dict[str, Any]


def build_container_prompt(
    *,
    instruction: str,
    container_id: str,
    workspace: str,
    user: str,
) -> str:
    """Tell Codex precisely where task work is allowed to happen.

    The task instruction is deliberately passed through verbatim.  It is the
    public benchmark task the agent is being scored on, not a controller
    instruction.  The surrounding contract prevents accidental edits in the
    host checkout where the CLI itself runs.
    """

    return f"""You are solving a benchmark task in one already-running Linux Docker container.

The target container is `{container_id}`.  Its task workspace is `{workspace}` and its normal task user is `{user}`.

Perform every investigation, edit, and test inside that exact target container.  Invoke commands using this form:

    docker exec --user {user} --workdir {workspace} {container_id} bash -lc '<command>'

Do not edit the host controller workspace, inspect other containers, or use host project files.  Do not merely describe a patch: make the intended changes in the target container and run focused checks when practical.  Work autonomously until the task is complete or you have exhausted useful debugging paths.

Task instruction:

{instruction}
"""


def run_container_task(
    *,
    instruction: str,
    container_id: str,
    workspace: str,
    user: str,
    controller_dir: Path,
    logs_dir: Path,
    timeout_sec: int,
    model: str | None = None,
    reasoning_effort: str | None = None,
) -> CodexRunResult:
    """Run one configured Codex turn and retain JSONL-only telemetry.

    When no explicit model is supplied, Codex uses the logged-in account's
    default.  Otherwise both the model and reasoning effort are passed as
    explicit CLI overrides.  The JSONL stream is persisted for audit, while
    all metadata returned to a harness excludes model and tool bodies.
    """

    if timeout_sec < 1:
        raise ValueError("timeout_sec must be positive")
    if not container_id.strip() or not workspace.strip() or not user.strip():
        raise ValueError("container id, workspace, and user are required")
    model = _optional_setting(model)
    reasoning_effort = _optional_setting(reasoning_effort)
    if bool(model) != bool(reasoning_effort):
        raise ValueError("model and reasoning_effort must be supplied together")

    final = _run_container_task_once(
        instruction=instruction,
        container_id=container_id,
        workspace=workspace,
        user=user,
        controller_dir=controller_dir,
        logs_dir=logs_dir,
        timeout_sec=timeout_sec,
        model=model,
        reasoning_effort=reasoning_effort,
    )
    telemetry = {
        **final.telemetry,
        "transportAttempts": 1,
    }
    return replace(final, telemetry=telemetry)


def _run_container_task_once(
    *,
    instruction: str,
    container_id: str,
    workspace: str,
    user: str,
    controller_dir: Path,
    logs_dir: Path,
    timeout_sec: int,
    model: str | None,
    reasoning_effort: str | None,
) -> CodexRunResult:
    """Perform one invocation; transport-only retries are managed above."""

    codex_command = _resolve_codex_command()

    controller_dir = _short_controller_dir(controller_dir)
    controller_dir.mkdir(parents=True, exist_ok=True)
    logs_dir.mkdir(parents=True, exist_ok=True)
    _ensure_git_repository(controller_dir)

    events_path = logs_dir / "codex-events.jsonl"
    stderr_path = logs_dir / "codex.stderr.log"
    final_path = logs_dir / "codex-final-message.txt"
    prompt = build_container_prompt(
        instruction=instruction,
        container_id=container_id,
        workspace=workspace,
        user=user,
    )
    (logs_dir / "codex-prompt.txt").write_text(prompt, encoding="utf-8")
    command = [
        *codex_command,
        "exec",
        "--json",
        "--ephemeral",
        "--sandbox",
        "danger-full-access",
        "--cd",
        str(controller_dir),
        "--output-last-message",
        str(final_path),
    ]
    if model:
        command.extend(["--model", model])
        command.extend(["--config", f'model_reasoning_effort="{reasoning_effort}"'])
    command.append(prompt)

    started = time.monotonic()
    started_at = datetime.now(timezone.utc).isoformat()
    timed_out = False
    return_code: int | None = None
    with events_path.open("wb") as stdout, stderr_path.open("wb") as stderr:
        process = subprocess.Popen(
            command,
            cwd=controller_dir,
            stdin=subprocess.DEVNULL,
            stdout=stdout,
            stderr=stderr,
        )
        try:
            return_code = process.wait(timeout=timeout_sec)
        except subprocess.TimeoutExpired:
            timed_out = True
            _terminate_process_tree(process)
            process.wait(timeout=60)
            return_code = process.returncode

    events = _read_jsonl(events_path)
    usage, telemetry = _summarize_events(events)
    turn_succeeded = bool(telemetry["turnCompleted"]) and not bool(
        telemetry["turnFailed"]
    )
    telemetry.update(
        {
            "executionRuntime": "local-codex-cli-to-official-container",
            "modelSelection": (
                f"codex-explicit-{model}"
                if model
                else "codex-account-default-no-model-flag"
            ),
            "reasoningEffort": reasoning_effort or "config-default",
            "modelSelectionMode": (
                "explicit-cli-overrides" if model else "no-explicit-cli-model-flag"
            ),
            "containerId": container_id,
            "workspace": workspace,
            "taskUser": user,
            "controllerDir": str(controller_dir),
            "eventsPath": str(events_path),
            "stderrPath": str(stderr_path),
            "finalMessagePath": str(final_path),
            "startedAtUtc": started_at,
            "durationSeconds": time.monotonic() - started,
            "controlledTimeout": timed_out,
            "exitCode": return_code,
        }
    )
    return CodexRunResult(
        return_code=return_code,
        timed_out=timed_out,
        turn_succeeded=turn_succeeded,
        usage=usage,
        telemetry=telemetry,
    )


def _optional_setting(value: str | None) -> str | None:
    """Normalize a CLI configuration value without broadening its meaning."""

    if value is None:
        return None
    normalized = value.strip()
    return normalized or None


def _ensure_git_repository(path: Path) -> None:
    if (path / ".git").exists():
        return
    result = subprocess.run(
        ["git", "init", "--quiet", str(path)],
        capture_output=True,
        text=True,
        timeout=30,
    )
    if result.returncode != 0:
        detail = (result.stderr or result.stdout).strip()
        raise RuntimeError(f"failed to initialize Codex controller repository: {detail}")


def _resolve_codex_command() -> list[str]:
    """Return an argv prefix that preserves multiline prompts on Windows.

    ``subprocess`` invokes ``codex.cmd`` through ``cmd.exe``.  Command Prompt
    treats embedded newlines as command separators before the wrapper's ``%*``
    expands, silently dropping the benchmark instruction.  Calling the same
    Node entrypoint directly keeps the prompt as one argv element.
    """

    codex = shutil.which("codex")
    if not codex:
        raise RuntimeError("local Codex CLI was not found on PATH")
    wrapper = Path(codex)
    if os.name != "nt" or wrapper.suffix.lower() not in {".cmd", ".bat"}:
        return [str(wrapper)]

    cli_entrypoint = wrapper.parent / "node_modules" / "@openai" / "codex" / "bin" / "codex.js"
    bundled_node = wrapper.parent / "node.exe"
    node = bundled_node if bundled_node.is_file() else Path(shutil.which("node") or "")
    if not node.is_file() or not cli_entrypoint.is_file():
        raise RuntimeError("could not resolve the installed Codex Node entrypoint")
    return [str(node), str(cli_entrypoint)]


def _short_controller_dir(requested: Path) -> Path:
    """Keep the empty controller Git repository below Windows path limits.

    Harbor nests each trial under a timestamp, task id, and random id.  Git
    appends sample-hook names during ``init``; that previously made a valid
    benchmark invocation fail before Codex started.  The controller contains
    no task source or result logs, so relocating just it by a stable hash does
    not alter the scored task environment.
    """

    resolved = requested.resolve()
    if os.name != "nt" or len(str(resolved)) < 120:
        return resolved
    digest = sha256(str(resolved).encode("utf-8")).hexdigest()[:24]
    return Path(tempfile.gettempdir()) / "opentopia-codex-controller" / digest


def _terminate_process_tree(process: subprocess.Popen[bytes]) -> None:
    """Stop only the CLI process tree that this helper started."""

    if process.poll() is not None:
        return
    if os.name == "nt":
        subprocess.run(
            ["taskkill", "/PID", str(process.pid), "/T", "/F"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=60,
            check=False,
        )
        return
    process.send_signal(signal.SIGTERM)
    try:
        process.wait(timeout=15)
    except subprocess.TimeoutExpired:
        process.kill()


def _read_jsonl(path: Path) -> list[dict[str, Any]]:
    events: list[dict[str, Any]] = []
    if not path.exists():
        return events
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(event, dict):
            events.append(event)
    return events


def _summarize_events(events: list[dict[str, Any]]) -> tuple[dict[str, int], dict[str, Any]]:
    usage = {
        "inputTokens": 0,
        "cachedInputTokens": 0,
        "outputTokens": 0,
        "reasoningTokens": 0,
    }
    telemetry: dict[str, Any] = {
        "eventCount": len(events),
        "turnCompleted": 0,
        "turnFailed": 0,
        "commandExecutionsStarted": 0,
        "commandExecutionsFinished": 0,
        "reasoningItemsCompleted": 0,
        "cacheTelemetry": "unsupported",
        "cliTransportWarnings": 0,
    }
    for event in events:
        event_type = event.get("type")
        if event_type == "turn.completed":
            telemetry["turnCompleted"] += 1
            _accumulate_usage(usage, event.get("usage"), telemetry)
        elif event_type == "turn.failed":
            telemetry["turnFailed"] += 1

        item = event.get("item")
        if not isinstance(item, dict):
            continue
        item_type = item.get("type")
        if event_type == "item.started" and item_type == "command_execution":
            telemetry["commandExecutionsStarted"] += 1
        elif event_type == "item.completed" and item_type == "command_execution":
            telemetry["commandExecutionsFinished"] += 1
        elif event_type == "item.completed" and item_type == "reasoning":
            telemetry["reasoningItemsCompleted"] += 1
    error_messages = [
        _event_error_message(event).lower()
        for event in events
        if _event_error_message(event)
    ]
    # The CLI can successfully fall back to HTTPS after WebSocket request-time
    # warnings.  Preserve their count for the audit, but do not discard a
    # completed run merely because this recoverable transport path occurred.
    telemetry["cliTransportWarnings"] = sum(
        "request timed out" in message or "falling back from websockets" in message
        for message in error_messages
    )
    return usage, telemetry


def _event_error_message(event: dict[str, Any]) -> str:
    if event.get("type") == "error" and isinstance(event.get("message"), str):
        return event["message"]
    item = event.get("item")
    if isinstance(item, dict) and item.get("type") == "error" and isinstance(
        item.get("message"), str
    ):
        return item["message"]
    return ""


def _accumulate_usage(
    target: dict[str, int], raw: Any, telemetry: dict[str, Any]
) -> None:
    if not isinstance(raw, dict):
        return
    target["inputTokens"] += int(raw.get("input_tokens") or 0)
    target["outputTokens"] += int(raw.get("output_tokens") or 0)
    target["reasoningTokens"] += int(raw.get("reasoning_output_tokens") or 0)
    if "cached_input_tokens" in raw:
        telemetry["cacheTelemetry"] = "codex-exec-reported"
        target["cachedInputTokens"] += int(raw.get("cached_input_tokens") or 0)
