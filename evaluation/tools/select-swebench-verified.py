#!/usr/bin/env python3
"""Create a reproducible, repo-balanced subset of SWE-bench Verified."""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import defaultdict
from datetime import datetime, timezone
from pathlib import Path

from datasets import load_dataset
from huggingface_hub import HfApi


def rank(seed: str, value: str) -> str:
    return hashlib.sha256(f"{seed}:{value}".encode("utf-8")).hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--count", type=int, default=60)
    parser.add_argument("--seed", default="opentopia-before-after-v1")
    parser.add_argument(
        "--revision",
        default=None,
        help="Optional Hugging Face dataset revision. Defaults to the current commit, which is recorded.",
    )
    args = parser.parse_args()
    if args.count < 1:
        raise SystemExit("--count must be positive")

    dataset_name = "SWE-bench/SWE-bench_Verified"
    dataset_info = HfApi().dataset_info(repo_id=dataset_name, revision=args.revision)
    dataset_revision = dataset_info.sha
    rows = list(load_dataset(dataset_name, revision=dataset_revision, split="test"))
    by_repo: dict[str, list[dict]] = defaultdict(list)
    for row in rows:
        by_repo[str(row["repo"])].append(dict(row))

    ordered_repositories = sorted(by_repo, key=lambda repo: rank(args.seed, repo))
    for repo, instances in by_repo.items():
        instances.sort(key=lambda instance: rank(args.seed, str(instance["instance_id"])))

    selected: list[dict] = []
    round_index = 0
    while len(selected) < args.count:
        added = 0
        for repo in ordered_repositories:
            instances = by_repo[repo]
            if round_index >= len(instances):
                continue
            selected.append(instances[round_index])
            added += 1
            if len(selected) == args.count:
                break
        if added == 0:
            break
        round_index += 1
    if len(selected) != args.count:
        raise SystemExit(f"requested {args.count} rows but selected {len(selected)}")

    instances = [
        {
            "instance_id": row["instance_id"],
            "repo": row["repo"],
            "base_commit": row["base_commit"],
            "version": row.get("version"),
        }
        for row in selected
    ]
    selection_fingerprint = hashlib.sha256(
        "\n".join(instance["instance_id"] for instance in instances).encode("utf-8")
    ).hexdigest()
    result = {
        "schemaVersion": 1,
        "dataset": dataset_name,
        "split": "test",
        "datasetRevision": dataset_revision,
        "datasetRevisionUrl": f"https://huggingface.co/datasets/{dataset_name}/tree/{dataset_revision}",
        "datasetRows": len(rows),
        "selectionMethod": "seeded repository round-robin; per-repository instance rank is SHA-256(seed:instance_id)",
        "seed": args.seed,
        "selectedAt": datetime.now(timezone.utc).isoformat(),
        "selectedCount": len(instances),
        "selectionFingerprintSha256": selection_fingerprint,
        "instances": instances,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"SWE_BENCH_SELECTION={args.output.resolve()}")
    print(f"SELECTION_FINGERPRINT={selection_fingerprint}")


if __name__ == "__main__":
    main()
