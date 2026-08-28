#!/usr/bin/env python3
"""Run local default-model Codex in one official SWE-bench task container."""

from __future__ import annotations

import argparse
import json
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

import docker

from evaluation.integrations.codex_cli import run_container_task


WORKSPACE = "/testbed"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--instances", required=True, type=Path)
    parser.add_argument("--instance-id", required=True)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--logs-dir", required=True, type=Path)
    parser.add_argument("--run-label", required=True)
    parser.add_argument("--run-timeout-seconds", type=int, default=1800)
    parser.add_argument("--prepare-only", action="store_true")
    return parser.parse_args()


def read_instance(path: Path, instance_id: str) -> dict[str, Any]:
    for line in path.read_text(encoding="utf-8").splitlines():
        row = json.loads(line)
        if row.get("instance_id") == instance_id:
            return row
    raise ValueError(f"instance id not found in materialized subset: {instance_id}")


def exec_checked(
    container: docker.models.containers.Container, command: str, *, timeout: int = 60
) -> str:
    del timeout  # docker-py exec_run does not expose a per-exec timeout.
    result = container.exec_run(
        ["/bin/sh", "-lc", command],
        workdir=WORKSPACE,
        demux=True,
    )
    stdout, stderr = result.output or (b"", b"")
    if result.exit_code != 0:
        detail = (stderr or stdout or b"no command output").decode(
            "utf-8", "replace"
        ).strip()
        raise RuntimeError(f"container command failed ({result.exit_code}): {detail[:1000]}")
    return (stdout or b"").decode("utf-8", "replace")


def run_agent(
    container: docker.models.containers.Container,
    instance: dict[str, Any],
    args: argparse.Namespace,
) -> tuple[str, dict[str, Any], dict[str, int]]:
    instruction = (
        "Solve the following SWE-bench issue in the repository at /testbed. "
        "Work directly in the repository, implement the fix, and run focused checks when practical. "
        "Do not only explain a solution; leave the intended code changes in the working tree.\n\n"
        + str(instance["problem_statement"])
    )
    started = time.monotonic()
    result = run_container_task(
        instruction=instruction,
        container_id=container.id,
        workspace=WORKSPACE,
        user="0:0",
        controller_dir=args.logs_dir / "codex-controller",
        logs_dir=args.logs_dir,
        timeout_sec=args.run_timeout_seconds,
    )
    controlled = {
        **result.telemetry,
        "turnStatus": "succeeded" if result.turn_succeeded else "failed",
        "turnError": None if result.turn_succeeded else "local Codex turn did not complete",
        "controlledTimeout": result.timed_out,
        "turnElapsedMs": int((time.monotonic() - started) * 1000),
    }
    diff = exec_checked(container, "git diff --binary --no-ext-diff")
    return diff, controlled, result.usage


def main() -> None:
    args = parse_args()
    if args.run_timeout_seconds < 1:
        raise SystemExit("run timeout must be positive")
    instance = read_instance(args.instances, args.instance_id)
    args.logs_dir.mkdir(parents=True, exist_ok=True)
    client = docker.from_env(timeout=600)
    try:
        image = client.images.get(str(instance["image"]))
    except docker.errors.ImageNotFound:
        image = client.images.pull(str(instance["image"]))
    safe_label = "".join(ch if ch.isalnum() else "-" for ch in args.run_label.lower())
    container_name = f"codex-swebench-{safe_label}-{args.instance_id.lower()}"
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
            agent_started_at = datetime.now(timezone.utc).isoformat()
            diff, controlled, usage = run_agent(container, instance, args)
            agent_finished_at = datetime.now(timezone.utc).isoformat()
            prediction = {
                "instance_id": args.instance_id,
                "model_name_or_path": "codex-account-default-no-model-flag",
                "model_patch": diff,
            }
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(json.dumps(prediction) + "\n", encoding="utf-8")
            record.update(
                {
                    "status": "completed" if controlled["turnStatus"] == "succeeded" else "agent_failed",
                    "controlledSettings": controlled,
                    "usage": usage,
                    "cacheTelemetry": controlled["cacheTelemetry"],
                    "eventCount": controlled["eventCount"],
                    "patchBytes": len(diff.encode("utf-8")),
                    "agentStartedAtUtc": agent_started_at,
                    "agentFinishedAtUtc": agent_finished_at,
                    "agentDurationSeconds": controlled["durationSeconds"],
                }
            )
    except Exception as error:
        record.update(
            {"status": "infrastructure_error", "errorType": type(error).__name__, "error": str(error)}
        )
        raise
    finally:
        args.output.with_suffix(".run.json").write_text(
            json.dumps(record, indent=2) + "\n", encoding="utf-8"
        )
        if container is not None:
            try:
                container.remove(force=True)
            except docker.errors.NotFound:
                pass


if __name__ == "__main__":
    main()
