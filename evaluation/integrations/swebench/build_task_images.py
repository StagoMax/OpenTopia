#!/usr/bin/env python3
"""Build official SWE-bench task images without Windows newline corruption.

SWE-bench v5 writes generated Dockerfiles using Python text mode.  On Windows
that rewrites LF into CRLF, which breaks Docker BuildKit heredocs in several
official task Dockerfiles.  This utility uses the official v5 ImageSpec and
task-repository loaders, then writes the exact generated Dockerfile bytes with
LF preserved before invoking the same `docker buildx build --load` command.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from swebench.image_builder.image_spec import get_image_specs_from_dataset
from swebench.task.repo import load_dockerfiles, task_paths


FRONTEND = "# syntax=docker/dockerfile:1.7\n"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--instances", required=True, type=Path)
    parser.add_argument("--task-repo", required=True, type=Path)
    parser.add_argument("--instance-id", action="append", required=True)
    parser.add_argument("--output-dir", required=True, type=Path)
    parser.add_argument("--force-rebuild", action="store_true")
    return parser.parse_args()


def load_instances(path: Path, wanted: set[str]) -> list[dict[str, Any]]:
    selected: dict[str, dict[str, Any]] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        row = json.loads(line)
        instance_id = row.get("instance_id")
        if instance_id in wanted:
            selected[instance_id] = row
    missing = sorted(wanted - selected.keys())
    if missing:
        raise ValueError(f"missing from materialized instance file: {', '.join(missing)}")
    return [selected[instance_id] for instance_id in sorted(wanted)]


def generated_dockerfile(spec: Any) -> bytes:
    dockerfile = spec.dockerfile
    if "\r" in dockerfile:
        raise ValueError(f"task Dockerfile for {spec.instance_id} was not checked out with LF")
    if not dockerfile.startswith("# syntax="):
        dockerfile = FRONTEND + dockerfile
    return dockerfile.encode("utf-8")


def build_one(spec: Any, output_dir: Path, force_rebuild: bool) -> dict[str, Any]:
    dockerfile_bytes = generated_dockerfile(spec)
    build_dir = output_dir / spec.filesafe_name
    build_dir.mkdir(parents=True, exist_ok=True)
    dockerfile_path = build_dir / "Dockerfile"
    log_path = build_dir / "build.log"
    dockerfile_path.write_bytes(dockerfile_bytes)

    command = [
        "docker",
        "buildx",
        "build",
        f"--platform={spec.platform}",
        f"--tag={spec.name}",
        f"--file={dockerfile_path}",
        "--progress=plain",
        "--load",
    ]
    if force_rebuild:
        command.append("--no-cache")
    command.append(str(spec.context_dir or dockerfile_path.parent))
    with log_path.open("w", encoding="utf-8", newline="\n") as log:
        completed = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, check=False)
    return {
        "instanceId": spec.instance_id,
        "image": spec.name,
        "platform": spec.platform,
        "context": str(spec.context_dir),
        "dockerfileSha256": hashlib.sha256(dockerfile_bytes).hexdigest(),
        "log": str(log_path),
        "exitCode": completed.returncode,
    }


def main() -> None:
    args = parse_args()
    wanted = set(args.instance_id)
    instances = load_instances(args.instances, wanted)
    dockerfiles = load_dockerfiles(args.task_repo, list(wanted))
    contexts = task_paths(args.task_repo, list(wanted))
    specs = get_image_specs_from_dataset(instances, dockerfiles, "swebench", "latest", contexts)
    by_id = {row["instance_id"]: row for row in instances}
    for spec in specs:
        declared_image = by_id[spec.instance_id].get("image")
        if declared_image != spec.name:
            raise ValueError(
                f"official image-name mismatch for {spec.instance_id}: {declared_image!r} != {spec.name!r}"
            )

    results = [build_one(spec, args.output_dir, args.force_rebuild) for spec in specs]
    report = {
        "schemaVersion": 1,
        "builder": "official-swebench-v5-imagespec + LF-compatible Dockerfile writer",
        "taskRepo": str(args.task_repo),
        "frontend": "docker/dockerfile:1.7",
        "generatedAt": datetime.now(timezone.utc).isoformat(),
        "results": results,
    }
    args.output_dir.mkdir(parents=True, exist_ok=True)
    (args.output_dir / "build-report.json").write_text(
        json.dumps(report, indent=2) + "\n", encoding="utf-8"
    )
    if any(result["exitCode"] != 0 for result in results):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
