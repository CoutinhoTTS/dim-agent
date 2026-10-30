#!/usr/bin/env python3
"""Check a corpus checkout against the manifest this repository pins.

The task corpora live in a private repository (arcships/dim-eval-data): part
of them may not be redistributed, and the OfficeQA tasks carry answer keys.
This repository keeps each corpus's MANIFEST.json, and a run uses a checkout
only when every task in it is exactly what that manifest names, digest for
digest, with nothing missing and nothing extra. A corpus change is therefore a
reviewed change here, and a task edited without regenerating its manifest is
caught instead of silently moving the measurement.

Usage: corpus.py --manifest benchmarks/tasks/MANIFEST.json --tasks-dir <checkout>/frontier
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent / "runner"))

from assemble_tasks import digest_dir  # noqa: E402


def verify(manifest_path: Path, tasks_dir: Path) -> list[str]:
    """What differs between ``tasks_dir`` and the pinned manifest; empty if nothing."""
    pinned = json.loads(manifest_path.read_text())
    tasks = pinned["tasks"]
    problems = []
    if hashlib.sha256(json.dumps(tasks, sort_keys=True).encode()).hexdigest() != pinned["tree_digest"]:
        problems.append(f"{manifest_path}: tree_digest does not match its own task list")
    copy = tasks_dir / "MANIFEST.json"
    if not copy.is_file() or json.loads(copy.read_text()) != pinned:
        problems.append(f"{copy} differs from the pinned {manifest_path}")
    for name, entry in sorted(tasks.items()):
        if not (tasks_dir / name).is_dir():
            problems.append(f"{name}: missing")
        elif digest_dir(tasks_dir / name) != entry["digest"]:
            problems.append(f"{name}: content differs from its pinned digest")
    extra = sorted(p.name for p in tasks_dir.iterdir()
                   if p.is_dir() and not p.name.startswith(".") and p.name not in tasks)
    problems += [f"{name}: not in the pinned manifest" for name in extra]
    return problems


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--tasks-dir", type=Path, required=True)
    args = parser.parse_args()
    problems = verify(args.manifest, args.tasks_dir)
    for problem in problems:
        print(f"[corpus] {problem}", file=sys.stderr)
    if problems:
        print(f"[corpus] {args.tasks_dir} is not the pinned corpus; refusing to run",
              file=sys.stderr)
        return 1
    print(f"[corpus] {args.tasks_dir} matches {args.manifest}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
