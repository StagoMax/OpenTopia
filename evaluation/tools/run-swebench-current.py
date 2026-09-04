#!/usr/bin/env python3
"""Run a resumable, current-snapshot SWE-bench Verified evaluation.

Each selected instance gets an isolated official image and an isolated attempt
directory.  Only attempts with a readable official scorer result are treated
as valid.  Provider, Docker, launcher, and adapter-invariant failures stop new
work instead of being silently converted to zero scores.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import os
import subprocess
import sys
import threading
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

import docker


PROVIDER_ERROR_MARKERS = (
    "provider request failed",
    "provider stream returned an error",
    "error sending request",
    "error decoding response body",
    "upstream_error",
)


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plan-root", required=True, type=Path)
    parser.add_argument("--instances", required=True, type=Path)
    parser.add_argument("--server-binary", required=True, type=Path)
    parser.add_argument("--env-file", required=True, type=Path)
    parser.add_argument("--runs-directory-name", required=True)
    parser.add_argument("--run-label-prefix", required=True)
    parser.add_argument(
        "--provider-label",
        required=True,
        help="Non-secret provider name recorded in every result and control-log event.",
    )
    parser.add_argument(
        "--model-label",
        required=True,
        help="Exact configured model name recorded separately from the redacted env file.",
    )
    parser.add_argument("--max-parallel", type=int, default=2, choices=range(1, 5))
    parser.add_argument("--max-parallel-scorers", type=int, default=2, choices=range(1, 5))
    parser.add_argument("--reasoning-effort", default="high")
    parser.add_argument("--max-output-tokens", type=int, default=8192)
    parser.add_argument("--agent-timeout-seconds", type=int, default=1800)
    parser.add_argument("--provider-test-timeout-seconds", type=int, default=180)
    parser.add_argument("--score-timeout-seconds", type=int, default=1800)
    parser.add_argument("--permission-mode", default="unrestricted", choices=("unrestricted",))
    parser.add_argument("--cleanup-images", action="store_true")
    parser.add_argument(
        "--instance-id",
        action="append",
        dest="instance_ids",
        help="Optional selected instance ID; repeat to run several. Defaults to every row.",
    )
    return parser.parse_args()


def read_instances(path: Path) -> list[dict[str, Any]]:
    rows = [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
    ids = [str(row.get("instance_id") or "") for row in rows]
    if not rows or any(not instance_id for instance_id in ids):
        raise RuntimeError("materialized subset was empty or contained an invalid instance ID")
    if len(ids) != len(set(ids)):
        raise RuntimeError("materialized subset contained duplicate instance IDs")
    return rows


def official_resolved(score: Any, instance_id: str) -> bool | None:
    if not isinstance(score, dict):
        return None
    result = score.get("result")
    if not isinstance(result, list) or len(result) != 2 or result[0] != instance_id:
        return None
    report = result[1]
    entry = report.get(instance_id) if isinstance(report, dict) else None
    resolved = entry.get("resolved") if isinstance(entry, dict) else None
    return resolved if isinstance(resolved, bool) else None


def load_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None


def provider_failure(record: dict[str, Any]) -> bool:
    settings = record.get("controlledSettings")
    settings = settings if isinstance(settings, dict) else {}
    error = settings.get("turnError")
    if not isinstance(error, str):
        return False
    normalized = error.lower()
    return "provider" in normalized or any(marker in normalized for marker in PROVIDER_ERROR_MARKERS)


def valid_attempt(attempt: Path, instance_id: str) -> tuple[bool, bool | None]:
    record = load_json(attempt / "prediction.run.json")
    score = load_json(attempt / "official-score.json")
    if not isinstance(record, dict) or record.get("status") == "infrastructure_error":
        return False, None
    if provider_failure(record):
        return False, None
    settings = record.get("controlledSettings")
    if not isinstance(settings, dict):
        return False, None
    if settings.get("permissionMode") != "unrestricted":
        return False, None
    if settings.get("approvalStrategy") != "none":
        return False, None
    if settings.get("automaticApprovals") != 0:
        return False, None
    resolved = official_resolved(score, instance_id)
    return resolved is not None, resolved


def existing_valid(instance_root: Path, instance_id: str) -> tuple[Path, bool] | None:
    for attempt in sorted(instance_root.glob("attempt-*"), reverse=True):
        valid, resolved = valid_attempt(attempt, instance_id)
        if valid and resolved is not None:
            return attempt, resolved
    return None


def next_attempt(instance_root: Path) -> Path:
    indexes: list[int] = []
    for path in instance_root.glob("attempt-*"):
        try:
            indexes.append(int(path.name.removeprefix("attempt-")))
        except ValueError:
            continue
    return instance_root / f"attempt-{max(indexes, default=0) + 1:03d}"


def append_event(path: Path, lock: threading.Lock, payload: dict[str, Any]) -> None:
    row = {"timestamp": utc_now(), **payload}
    with lock:
        with path.open("a", encoding="utf-8", newline="\n") as stream:
            stream.write(json.dumps(row, ensure_ascii=False) + "\n")


def write_json_atomic(path: Path, payload: dict[str, Any]) -> None:
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(payload, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    os.replace(temporary, path)


def run_command(command: list[str], stdout_path: Path, stderr_path: Path, timeout: int) -> int:
    with stdout_path.open("ab") as stdout, stderr_path.open("ab") as stderr:
        try:
            completed = subprocess.run(command, stdout=stdout, stderr=stderr, timeout=timeout, check=False)
            return completed.returncode
        except subprocess.TimeoutExpired:
            stderr.write(f"outer launcher timeout after {timeout} seconds\n".encode())
            return 124


def cleanup_image(image_name: str) -> str:
    try:
        client = docker.from_env(timeout=600)
        client.images.remove(image=image_name, force=False, noprune=False)
        return "removed"
    except docker.errors.ImageNotFound:
        return "already_absent"
    except Exception as error:  # Cleanup is recorded but never invalidates an official result.
        return f"failed:{type(error).__name__}"


def main() -> None:
    args = parse_args()
    source_root = Path(__file__).resolve().parents[2]
    agent_runner = source_root / "evaluation" / "integrations" / "swebench" / "run_opentopia_instance.py"
    scorer = source_root / "evaluation" / "integrations" / "swebench" / "score_opentopia_prediction.py"
    for path in (args.plan_root, args.instances, args.server_binary, args.env_file, agent_runner, scorer):
        if not path.exists():
            raise SystemExit(f"required evaluation input was missing: {path}")

    client = docker.from_env(timeout=20)
    client.ping()
    client.close()

    rows = read_instances(args.instances)
    by_id = {str(row["instance_id"]): row for row in rows}
    selected_ids = args.instance_ids or list(by_id)
    missing = [instance_id for instance_id in selected_ids if instance_id not in by_id]
    if missing:
        raise SystemExit(f"requested instance IDs were absent from the subset: {missing}")

    runs_root = args.plan_root / args.runs_directory_name
    runs_root.mkdir(parents=True, exist_ok=True)
    run_log = runs_root / "control-log.jsonl"
    manifest_path = runs_root / "manifest.json"
    segment_path = runs_root / "provider-segment.json"
    lock = threading.Lock()
    scorer_slots = threading.Semaphore(args.max_parallel_scorers)
    stop = threading.Event()
    results: dict[str, dict[str, Any]] = {}
    pending: list[tuple[str, dict[str, Any]]] = []

    write_json_atomic(
        segment_path,
        {
            "schemaVersion": 1,
            "createdAtUtc": utc_now(),
            "providerLabel": args.provider_label,
            "model": args.model_label,
            "reasoningEffort": args.reasoning_effort,
            "permissionMode": args.permission_mode,
            "approvalStrategy": "none",
            "runLabelPrefix": args.run_label_prefix,
            "instancesFile": str(args.instances.resolve()),
            "selectedInstanceIds": selected_ids,
            "credentialMaterial": "redacted-env-file",
        },
    )

    for instance_id in selected_ids:
        found = existing_valid(runs_root / instance_id, instance_id)
        if found:
            results[instance_id] = {
                "status": "valid_existing",
                "attempt": str(found[0]),
                "resolved": found[1],
            }
        else:
            pending.append((instance_id, by_id[instance_id]))

    def run_one(item: tuple[str, dict[str, Any]]) -> dict[str, Any]:
        instance_id, instance = item
        if stop.is_set():
            return {"status": "not_started_after_failure"}
        attempt = next_attempt(runs_root / instance_id)
        attempt.mkdir(parents=True, exist_ok=False)
        prediction = attempt / "prediction.jsonl"
        run_label = f"{args.run_label_prefix}-{instance_id}"
        common = {
            "instanceId": instance_id,
            "attempt": attempt.name,
            "providerLabel": args.provider_label,
            "model": args.model_label,
            "reasoningEffort": args.reasoning_effort,
            "permissionMode": args.permission_mode,
            "approvalStrategy": "none",
        }
        write_json_atomic(
            attempt / "provider-provenance.json",
            {
                "schemaVersion": 1,
                "recordedAtUtc": utc_now(),
                "providerLabel": args.provider_label,
                "model": args.model_label,
                "reasoningEffort": args.reasoning_effort,
                "permissionMode": args.permission_mode,
                "approvalStrategy": "none",
                "runLabel": run_label,
                "credentialMaterial": "redacted-env-file",
            },
        )
        append_event(run_log, lock, {**common, "stage": "agent_started"})
        agent_command = [
            sys.executable,
            str(agent_runner),
            "--instances", str(args.instances),
            "--instance-id", instance_id,
            "--server-binary", str(args.server_binary),
            "--env-file", str(args.env_file),
            "--output", str(prediction),
            "--logs-dir", str(attempt / "agent-logs"),
            "--run-label", run_label,
            "--reasoning-effort", args.reasoning_effort,
            "--max-output-tokens", str(args.max_output_tokens),
            "--run-timeout-seconds", str(args.agent_timeout_seconds),
            "--provider-test-timeout-seconds", str(args.provider_test_timeout_seconds),
            "--permission-mode", args.permission_mode,
            "--approval-strategy", "none",
        ]
        agent_exit = run_command(
            agent_command,
            attempt / "agent.stdout.log",
            attempt / "agent.stderr.log",
            args.agent_timeout_seconds + 600,
        )
        append_event(run_log, lock, {**common, "stage": "agent_finished", "exitCode": agent_exit})
        record = load_json(attempt / "prediction.run.json")
        invalid_reason = None
        if agent_exit != 0:
            invalid_reason = f"agent_exit_{agent_exit}"
        elif not isinstance(record, dict):
            invalid_reason = "missing_run_record"
        elif record.get("status") == "infrastructure_error":
            invalid_reason = "adapter_or_docker_infrastructure_error"
        elif provider_failure(record):
            invalid_reason = "provider_failure"
        else:
            settings = record.get("controlledSettings")
            if not isinstance(settings, dict):
                invalid_reason = "missing_controlled_settings"
            elif settings.get("permissionMode") != "unrestricted":
                invalid_reason = "permission_invariant_failed"
            elif settings.get("approvalStrategy") != "none" or settings.get("automaticApprovals") != 0:
                invalid_reason = "approval_invariant_failed"
        if invalid_reason:
            stop.set()
            append_event(run_log, lock, {**common, "stage": "invalid", "reason": invalid_reason})
            return {"status": "invalid", "reason": invalid_reason, "attempt": str(attempt)}

        append_event(run_log, lock, {**common, "stage": "score_started"})
        score_command = [
            sys.executable,
            "-X",
            "utf8",
            str(scorer),
            "--instances", str(args.instances),
            "--instance-id", instance_id,
            "--prediction", str(prediction),
            "--run-id", run_label,
            "--timeout-seconds", str(args.score_timeout_seconds),
            "--output", str(attempt / "official-score.json"),
        ]
        with scorer_slots:
            score_exit = run_command(
                score_command,
                attempt / "score.stdout.log",
                attempt / "score.stderr.log",
                args.score_timeout_seconds + 600,
            )
        score = load_json(attempt / "official-score.json")
        resolved = official_resolved(score, instance_id)
        append_event(
            run_log,
            lock,
            {**common, "stage": "score_finished", "exitCode": score_exit, "officialResult": resolved is not None},
        )
        if score_exit != 0 or resolved is None:
            stop.set()
            return {
                "status": "invalid",
                "reason": f"official_score_exit_{score_exit}" if score_exit else "invalid_official_score",
                "attempt": str(attempt),
            }
        cleanup = cleanup_image(str(instance["image"])) if args.cleanup_images else "disabled"
        append_event(run_log, lock, {**common, "stage": "valid", "resolved": resolved, "imageCleanup": cleanup})
        return {"status": "valid", "resolved": resolved, "attempt": str(attempt), "imageCleanup": cleanup}

    with concurrent.futures.ThreadPoolExecutor(max_workers=args.max_parallel) as executor:
        futures = {executor.submit(run_one, item): item[0] for item in pending}
        for future in concurrent.futures.as_completed(futures):
            instance_id = futures[future]
            try:
                results[instance_id] = future.result()
            except Exception as error:
                stop.set()
                results[instance_id] = {"status": "invalid", "reason": type(error).__name__}

    ordered = {instance_id: results.get(instance_id, {"status": "not_started"}) for instance_id in selected_ids}
    manifest = {
        "schemaVersion": 2,
        "updatedAtUtc": utc_now(),
        "instancesFile": str(args.instances.resolve()),
        "serverBinary": str(args.server_binary.resolve()),
        "runLabelPrefix": args.run_label_prefix,
        "providerLabel": args.provider_label,
        "model": args.model_label,
        "reasoningEffort": args.reasoning_effort,
        "permissionMode": args.permission_mode,
        "approvalStrategy": "none",
        "maxParallel": args.max_parallel,
        "maxParallelScorers": args.max_parallel_scorers,
        "providerTestTimeoutSeconds": args.provider_test_timeout_seconds,
        "cleanupImages": args.cleanup_images,
        "selectedCount": len(selected_ids),
        "validCount": sum(item.get("status") in {"valid", "valid_existing"} for item in ordered.values()),
        "invalidCount": sum(item.get("status") == "invalid" for item in ordered.values()),
        "results": ordered,
    }
    temporary = manifest_path.with_suffix(".json.tmp")
    temporary.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    os.replace(temporary, manifest_path)
    print(json.dumps({key: manifest[key] for key in ("selectedCount", "validCount", "invalidCount")}, ensure_ascii=False))
    if manifest["invalidCount"]:
        raise SystemExit(2)


if __name__ == "__main__":
    main()
