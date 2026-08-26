#!/usr/bin/env python3
"""Materialize a pinned SWE-bench selection for the official local harness."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

from datasets import load_dataset


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--selection", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()

    selection = json.loads(args.selection.read_text(encoding="utf-8"))
    required = {"dataset", "datasetRevision", "split", "instances"}
    missing = required - selection.keys()
    if missing:
        raise SystemExit(f"selection is missing required fields: {sorted(missing)}")

    selected_ids = [row["instance_id"] for row in selection["instances"]]
    if len(selected_ids) != len(set(selected_ids)):
        raise SystemExit("selection contains duplicate instance IDs")

    dataset = load_dataset(
        selection["dataset"],
        revision=selection["datasetRevision"],
        split=selection["split"],
    )
    by_id = {str(row["instance_id"]): dict(row) for row in dataset}
    missing_ids = [instance_id for instance_id in selected_ids if instance_id not in by_id]
    if missing_ids:
        raise SystemExit(f"pinned dataset is missing selected IDs: {missing_ids}")

    rows = [by_id[instance_id] for instance_id in selected_ids]
    output = args.output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(
        "".join(json.dumps(row, ensure_ascii=False) + "\n" for row in rows),
        encoding="utf-8",
    )
    digest = hashlib.sha256(output.read_bytes()).hexdigest()
    print(f"SWE_BENCH_SUBSET={output}")
    print(f"ROWS={len(rows)}")
    print(f"SHA256={digest}")


if __name__ == "__main__":
    main()
