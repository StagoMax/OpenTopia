#!/usr/bin/env python3
"""Grade one emitted OpenTopia patch with the official SWE-bench 5.x runner."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

import docker
import swebench
from swebench.harness.run_evaluation import run_instance
from swebench.harness.utils import make_test_spec


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--instances", required=True, type=Path)
    parser.add_argument("--instance-id", required=True)
    parser.add_argument("--prediction", required=True, type=Path)
    parser.add_argument("--run-id", required=True)
    parser.add_argument("--timeout-seconds", type=int, default=1800)
    parser.add_argument("--output", required=True, type=Path)
    return parser.parse_args()


def find_jsonl_row(path: Path, instance_id: str) -> dict:
    for line in path.read_text(encoding="utf-8").splitlines():
        row = json.loads(line)
        if row.get("instance_id") == instance_id:
            return row
    raise ValueError(f"{instance_id} is absent from {path}")


def main() -> None:
    args = parse_args()
    instance = find_jsonl_row(args.instances, args.instance_id)
    prediction = find_jsonl_row(args.prediction, args.instance_id)
    result = run_instance(
        make_test_spec(instance),
        prediction,
        docker.from_env(timeout=600),
        args.run_id,
        timeout=args.timeout_seconds,
    )
    record = {
        "schemaVersion": 1,
        "officialHarness": "SWE-bench 5.0.2 run_instance",
        "harnessVersion": swebench.__version__,
        "instanceId": args.instance_id,
        "runId": args.run_id,
        "result": result,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")
    if result is None:
        raise SystemExit("official SWE-bench run_instance did not produce a report")


if __name__ == "__main__":
    main()
