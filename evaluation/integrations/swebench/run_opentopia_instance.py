#!/usr/bin/env python3
"""Run one OpenTopia snapshot in an official SWE-bench instance container.

The agent operates in the exact `/testbed` repository image that SWE-bench
later uses to apply and grade its emitted patch.  The official SWE-bench
grader is intentionally invoked separately, against the JSONL prediction this
script produces.
"""

from __future__ import annotations

import argparse
import base64
import io
import json
import os
import secrets
import sys
import tarfile
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from dotenv import dotenv_values

import docker


RUNTIME_DIR = "/tmp/opentopia-eval"
SERVER_PATH = f"{RUNTIME_DIR}/opentopia-server"
SERVER_LOG_PATH = f"{RUNTIME_DIR}/server.log"
SERVER_DB_PATH = f"{RUNTIME_DIR}/opentopia.db"
SERVER_PORT = 18787
WORKSPACE = "/testbed"
ENV_ALIASES = {
    "OPENTOPIA_EVAL_API_KEY": "AUDIT_COPILOT_LLM_API_KEY",
    "OPENTOPIA_EVAL_BASE_URL": "AUDIT_COPILOT_LLM_BASE_URL",
    "OPENTOPIA_EVAL_MODEL": "AUDIT_COPILOT_LLM_MODEL",
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--instances", required=True, type=Path)
    parser.add_argument("--instance-id", required=True)
    parser.add_argument("--server-binary", required=True, type=Path)
    parser.add_argument("--env-file", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--logs-dir", required=True, type=Path)
    parser.add_argument("--run-label", required=True)
    parser.add_argument("--reasoning-effort", default="high")
    parser.add_argument(
        "--permission-mode",
        choices=("full-access", "unrestricted"),
        default="full-access",
        help="OpenTopia permission preset used for the evaluated turn.",
    )
    parser.add_argument(
        "--approval-strategy",
        choices=("external-auto-approve", "none"),
        default="external-auto-approve",
        help=(
            "How the benchmark adapter handles product approval requests. "
            "Use none with unrestricted to avoid reviewer or simulated-user overhead."
        ),
    )
    parser.add_argument("--max-output-tokens", type=int, default=8192)
    parser.add_argument("--rollout-limit-tokens", type=int)
    parser.add_argument("--run-timeout-seconds", type=int, default=1800)
    parser.add_argument("--provider-test-timeout-seconds", type=int, default=180)
    parser.add_argument("--prepare-only", action="store_true")
    return parser.parse_args()


def read_instance(path: Path, instance_id: str) -> dict[str, Any]:
    for line in path.read_text(encoding="utf-8").splitlines():
        row = json.loads(line)
        if row.get("instance_id") == instance_id:
            return row
    raise ValueError(f"instance id not found in materialized subset: {instance_id}")


def required_env(values: dict[str, str | None], name: str) -> str:
    value = values.get(name) or values.get(ENV_ALIASES[name])
    if not value:
        raise RuntimeError(f"missing {name} in evaluation env file")
    return value


def exec_checked(container: docker.models.containers.Container, command: str, *, env: dict[str, str] | None = None, timeout: int = 60) -> str:
    result = container.exec_run(
        ["/bin/sh", "-lc", command],
        workdir=WORKSPACE,
        environment=env,
        demux=True,
    )
    stdout, stderr = result.output or (b"", b"")
    if result.exit_code != 0:
        detail = (stderr or stdout or b"no command output").decode("utf-8", "replace").strip()
        raise RuntimeError(f"container command failed ({result.exit_code}): {detail[:1000]}")
    return (stdout or b"").decode("utf-8", "replace")


def upload_server(container: docker.models.containers.Container, binary: Path) -> None:
    payload = io.BytesIO()
    with tarfile.open(fileobj=payload, mode="w") as archive:
        info = tarfile.TarInfo("opentopia-server")
        info.mode = 0o755
        info.size = binary.stat().st_size
        with binary.open("rb") as file:
            archive.addfile(info, file)
    exec_checked(container, f"mkdir -p {RUNTIME_DIR} && chmod 777 {RUNTIME_DIR}")
    if not container.put_archive(RUNTIME_DIR, payload.getvalue()):
        raise RuntimeError("failed to upload OpenTopia server binary")


def api(
    container: docker.models.containers.Container,
    token: str,
    method: str,
    path: str,
    body: dict[str, Any] | None = None,
    *,
    timeout_seconds: int = 30,
) -> Any:
    encoded_body = base64.b64encode(
        b"" if body is None else json.dumps(body, separators=(",", ":")).encode("utf-8")
    ).decode("ascii")
    script = "\n".join(
        [
            "import base64, os, sys, urllib.request",
            f"data = base64.b64decode({encoded_body!r}) if {body is not None!r} else None",
            "headers = {'authorization': 'Bearer ' + os.environ['OPENTOPIA_EVAL_INTERNAL_TOKEN'], 'content-type': 'application/json'}",
            f"request = urllib.request.Request('http://127.0.0.1:{SERVER_PORT}{path}', data=data, headers=headers, method={method!r})",
            f"with urllib.request.urlopen(request, timeout={timeout_seconds}) as response:",
            "    sys.stdout.write(response.read().decode('utf-8'))",
        ]
    )
    encoded_script = base64.b64encode(script.encode("utf-8")).decode("ascii")
    output = exec_checked(
        container,
        f"printf %s {encoded_script} | base64 -d | python3",
        env={"OPENTOPIA_EVAL_INTERNAL_TOKEN": token},
    )
    try:
        return json.loads(output)
    except json.JSONDecodeError as error:
        raise RuntimeError(f"OpenTopia API {method} {path} returned non-JSON") from error


def configure_server(
    container: docker.models.containers.Container,
    config: dict[str, str | None],
    args: argparse.Namespace,
) -> tuple[str, dict[str, Any]]:
    token = secrets.token_urlsafe(36)
    server_env = {
        "OPENTOPIA_API_KEY": required_env(config, "OPENTOPIA_EVAL_API_KEY"),
        "OPENTOPIA_OPENAI_BASE_URL": required_env(config, "OPENTOPIA_EVAL_BASE_URL"),
        "OPENTOPIA_MODEL": required_env(config, "OPENTOPIA_EVAL_MODEL"),
        "OPENTOPIA_API_TOKEN": token,
        "OPENTOPIA_DB": SERVER_DB_PATH,
        "OPENTOPIA_PERMISSION": args.permission_mode,
        "OPENTOPIA_SANDBOX_MODE": "danger-full-access",
        "OPENTOPIA_SANDBOX_ENFORCEMENT": "disabled",
        "OPENTOPIA_SANDBOX_NETWORK": "inherit",
    }
    exec_checked(
        container,
        f"nohup {SERVER_PATH} --host 127.0.0.1 --port {SERVER_PORT} --db {SERVER_DB_PATH} --permission {args.permission_mode} > {SERVER_LOG_PATH} 2>&1 < /dev/null &",
        env=server_env,
    )
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        try:
            health = api(container, token, "GET", "/health")
            if health.get("ok") is True and health.get("service") == "opentopia-server":
                break
        except RuntimeError:
            time.sleep(1)
    else:
        raise RuntimeError("OpenTopia server did not become healthy")

    settings = api(container, token, "GET", "/api/settings")
    provider_id = settings.get("activeProviderId")
    providers = settings.get("providers") or []
    provider = next((item for item in providers if item.get("id") == provider_id), None)
    if not provider:
        raise RuntimeError("active provider was missing from OpenTopia settings")
    provider["model"] = required_env(config, "OPENTOPIA_EVAL_MODEL")
    provider["reasoningEffort"] = args.reasoning_effort
    provider["maxOutputTokens"] = args.max_output_tokens
    provider["rolloutBudget"] = (
        {
            "limitTokens": args.rollout_limit_tokens,
            "samplingTokenWeight": 1.0,
            "prefillTokenWeight": 1.0,
        }
        if args.rollout_limit_tokens is not None
        else None
    )
    api(container, token, "PATCH", "/api/settings", {"providers": providers, "activeProviderId": provider_id})
    profile = api(
        container,
        token,
        "POST",
        "/api/provider/test",
        {"providerId": provider_id},
        timeout_seconds=args.provider_test_timeout_seconds,
    )
    if not profile.get("reachable") or not profile.get("modelAvailable"):
        raise RuntimeError("provider capability negotiation did not confirm the model")
    return token, {
        "executionRuntime": "official-swebench-instance-container",
        "workspace": WORKSPACE,
        "permissionMode": args.permission_mode,
        "approvalStrategy": args.approval_strategy,
        "model": required_env(config, "OPENTOPIA_EVAL_MODEL"),
        "reasoningEffort": args.reasoning_effort,
        "maxOutputTokens": args.max_output_tokens,
        "rolloutLimitTokens": args.rollout_limit_tokens,
        "providerCapabilityProfileTested": True,
        "providerTestTimeoutSeconds": args.provider_test_timeout_seconds,
    }


def usage_from_events(events: list[dict[str, Any]]) -> tuple[dict[str, int], bool]:
    usage = {"inputTokens": 0, "cachedInputTokens": 0, "outputTokens": 0, "reasoningTokens": 0}
    cache_reported = False
    for event in events:
        payload = event.get("payload") or {}
        if payload.get("type") != "token_usage":
            continue
        usage["inputTokens"] += int(payload.get("input_tokens") or 0)
        usage["outputTokens"] += int(payload.get("output_tokens") or 0)
        usage["reasoningTokens"] += int(payload.get("reasoning_tokens") or 0)
        if "cached_input_tokens" in payload:
            cache_reported = True
            usage["cachedInputTokens"] += int(payload.get("cached_input_tokens") or 0)
    return usage, cache_reported


def event_metrics(events: list[dict[str, Any]]) -> dict[str, int]:
    event_types = [
        (event.get("payload") or {}).get("type")
        for event in events
        if isinstance(event, dict)
    ]
    return {
        "modelRequests": sum(event_type == "model_request" for event_type in event_types),
        "providerResponses": sum(
            event_type == "provider_response_received" for event_type in event_types
        ),
        "toolCallsStarted": sum(event_type == "tool_call_started" for event_type in event_types),
        "toolCallsFinished": sum(
            event_type == "tool_call_finished" for event_type in event_types
        ),
    }


def run_agent(
    container: docker.models.containers.Container,
    instance: dict[str, Any],
    config: dict[str, str | None],
    args: argparse.Namespace,
) -> tuple[str, dict[str, Any], list[dict[str, Any]]]:
    token, controlled = configure_server(container, config, args)
    thread = api(container, token, "POST", "/api/threads", {"title": args.run_label, "workspaceRoot": WORKSPACE})
    thread_id = thread.get("id")
    if not isinstance(thread_id, str) or not thread_id:
        raise RuntimeError("OpenTopia did not create an evaluation thread")
    instruction = (
        "Solve the following SWE-bench issue in the repository at /testbed. "
        "Work directly in the repository, implement the fix, and run focused checks when practical. "
        "Do not only explain a solution; leave the intended code changes in the working tree.\n\n"
        + str(instance["problem_statement"])
    )
    api(container, token, "POST", f"/api/threads/{thread_id}/messages", {"content": instruction})
    deadline = time.monotonic() + args.run_timeout_seconds
    approvals = 0
    terminal: dict[str, Any] | None = None
    last_candidate: dict[str, Any] | None = None
    controlled_timeout = False
    while time.monotonic() < deadline:
        candidate = api(container, token, "GET", f"/api/threads/{thread_id}/turn")
        if isinstance(candidate, dict):
            last_candidate = candidate
        status = candidate.get("status") if candidate else None
        if status in {"succeeded", "failed", "cancelled", "interrupted", "waiting_user_action"}:
            terminal = candidate
            break
        if status == "waiting_approval":
            if args.approval_strategy == "none":
                raise RuntimeError(
                    "turn requested approval while the evaluation approval strategy was none"
                )
            pending = api(container, token, "GET", f"/api/threads/{thread_id}/approvals?status=pending")
            for approval in pending:
                approval_id = approval.get("approvalId")
                if not approval_id:
                    raise RuntimeError("OpenTopia returned an invalid approval")
                api(container, token, "POST", f"/api/threads/{thread_id}/approvals/{approval_id}/decision", {"approved": True})
                approvals += 1
        time.sleep(1)
    if terminal is None:
        controlled_timeout = True
        api(container, token, "POST", f"/api/threads/{thread_id}/turn/cancel", {})
        for _ in range(10):
            candidate = api(container, token, "GET", f"/api/threads/{thread_id}/turn")
            if isinstance(candidate, dict):
                last_candidate = candidate
            if candidate and candidate.get("status") in {
                "succeeded",
                "failed",
                "cancelled",
                "interrupted",
            }:
                terminal = candidate
                break
            time.sleep(1)
        if terminal is None:
            terminal = last_candidate or {
                "status": "timed_out",
                "error": "turn status unavailable after controlled timeout",
            }
    # The conversation projection keeps the event types and token-usage fields
    # needed for scoring while omitting large model/tool payloads.  This avoids
    # a long agent run failing during post-run telemetry collection.
    events = api(
        container,
        token,
        "GET",
        f"/api/threads/{thread_id}/events?since=0&view=conversation",
        timeout_seconds=300,
    )
    if not isinstance(events, list):
        raise RuntimeError("OpenTopia events response was not a list")
    controlled.update({
        "threadId": thread_id,
        "turnStatus": terminal.get("status"),
        "turnError": terminal.get("error"),
        "automaticApprovals": approvals,
        "controlledTimeout": controlled_timeout,
        "rolloutLimitTokens": args.rollout_limit_tokens,
        **event_metrics(events),
    })
    diff = exec_checked(container, "git diff --binary --no-ext-diff")
    return diff, controlled, events


def main() -> None:
    args = parse_args()
    if args.permission_mode == "unrestricted" and args.approval_strategy != "none":
        raise SystemExit(
            "--permission-mode unrestricted requires --approval-strategy none"
        )
    if not args.server_binary.is_file():
        raise SystemExit(f"server binary not found: {args.server_binary}")
    instance = read_instance(args.instances, args.instance_id)
    args.logs_dir.mkdir(parents=True, exist_ok=True)
    client = docker.from_env(timeout=600)
    try:
        image = client.images.get(str(instance["image"]))
    except docker.errors.ImageNotFound:
        image = client.images.pull(str(instance["image"]))
    safe_label = "".join(ch if ch.isalnum() else "-" for ch in args.run_label.lower())
    container_name = f"opentopia-swebench-{safe_label}-{args.instance_id.lower()}"
    container = None
    record: dict[str, Any] = {
        "instanceId": args.instance_id,
        "runLabel": args.run_label,
        "status": "prepared" if args.prepare_only else "running",
        "controlledSettings": None,
        "officialInstanceImage": str(instance["image"]),
        "pulledImageId": image.id,
        "pulledImageRepoDigests": image.attrs.get("RepoDigests") or [],
    }
    try:
        try:
            existing = client.containers.get(container_name)
            existing.remove(force=True)
        except docker.errors.NotFound:
            pass
        container = client.containers.create(
            image=image.id,
            name=container_name,
            command="tail -f /dev/null",
            detach=True,
        )
        container.start()
        if args.prepare_only:
            exec_checked(container, "test -d /testbed && git rev-parse --is-inside-work-tree")
            record["status"] = "prepared"
        else:
            upload_server(container, args.server_binary)
            env_values = dict(dotenv_values(args.env_file)) if args.env_file else {}
            agent_started = time.monotonic()
            agent_started_at = datetime.now(timezone.utc).isoformat()
            diff, controlled, events = run_agent(container, instance, env_values, args)
            agent_finished_at = datetime.now(timezone.utc).isoformat()
            usage, cache_reported = usage_from_events(events)
            args.logs_dir.joinpath("opentopia-events.json").write_text(json.dumps(events, indent=2) + "\n", encoding="utf-8")
            prediction = {
                "instance_id": args.instance_id,
                "model_name_or_path": args.run_label,
                "model_patch": diff,
            }
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(json.dumps(prediction) + "\n", encoding="utf-8")
            record.update({
                "status": "completed" if controlled["turnStatus"] == "succeeded" else "agent_failed",
                "controlledSettings": controlled,
                "usage": usage,
                "cacheTelemetry": "provider_reported" if cache_reported else "unsupported",
                "eventCount": len(events),
                "patchBytes": len(diff.encode("utf-8")),
                "agentStartedAtUtc": agent_started_at,
                "agentFinishedAtUtc": agent_finished_at,
                "agentDurationSeconds": time.monotonic() - agent_started,
            })
    except Exception as error:
        record.update({"status": "infrastructure_error", "errorType": type(error).__name__, "error": str(error)})
        raise
    finally:
        args.output.with_suffix(".run.json").write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")
        if container is not None:
            try:
                container.remove(force=True)
            except docker.errors.NotFound:
                pass


if __name__ == "__main__":
    main()
