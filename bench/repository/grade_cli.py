"""Run the frozen repository grader without importing the mutable worktree."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from bench.repository.grading import grade  # noqa: E402


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixture", type=Path, required=True)
    parser.add_argument("--work", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--timeout", type=float, required=True)
    args = parser.parse_args()
    language = json.loads((args.fixture / "inventory.json").read_text()).get("language", "python")
    if language == "go":
        from bench.repository.go_fixture import grade as grade_go

        return grade_go(args.fixture, args.work, args.output, args.timeout)
    if language != "python":
        parser.error("Unsupported frozen repository language.")
    return grade(args.fixture, args.work, args.output, args.timeout)


if __name__ == "__main__":
    sys.exit(main())
