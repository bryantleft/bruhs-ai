"""Materialize a repository fixture from its pinned, checksum-verified archive."""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path
from typing import TypedDict

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
from bench.lib.test_receipts import FrozenTests  # noqa: E402


class Inventory(TypedDict):
    upstream_tests: list[str]
    contract_tests: list[str]
    groups: list[list[str]]
    environment: dict[str, str]
    scope: str
    source: dict[str, object]
    excluded_tests: dict[str, str]
    sqlite_case_isolation: str


def prepare(definition: Path, archive: Path, output: Path) -> Inventory:
    source = json.loads((definition / "source.json").read_text())
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    if digest != source["archive_sha256"]:
        raise ValueError("Upstream archive checksum does not match the pinned fixture.")
    output.mkdir(parents=True, exist_ok=False)
    with tarfile.open(archive) as package:
        for member in package.getmembers():
            if (
                not (member.isfile() or member.isdir())
                or Path(member.name).is_absolute()
                or ".." in Path(member.name).parts
            ):
                raise ValueError(
                    "Upstream archive contains an unsupported path or link."
                )
        package.extractall(output / "unpack", filter="data")
    unpack = output / "unpack"
    extracted = list(unpack.iterdir())
    if len(extracted) != 1 or not extracted[0].is_dir():
        raise ValueError("Pinned archive must contain one repository root.")
    repo = output / "repo"
    extracted[0].rename(repo)
    unpack.rmdir()
    (repo / "tests").mkdir(exist_ok=True)
    shutil.copy2(
        definition / "visible_tests.py", repo / "tests/test_leased_delivery.py"
    )
    shutil.copy2(definition / "prompt.md", repo / "LEASED_DELIVERY_SPEC.md")
    shutil.copytree(definition / "hidden", output / "hidden")
    for name in (
        "source.json",
        "prompt.md",
        "regressions.json",
        "reference.diff",
        "campaign.json",
    ):
        shutil.copy2(definition / name, output / name)
    config = json.loads((definition / "regressions.json").read_text())
    # Inventory only the trusted pinned baseline, never a submitted tree.
    collect = """import json,os,sys,unittest
sys.path.insert(0,sys.argv[1])
suite=unittest.defaultTestLoader.loadTestsFromNames(json.loads(sys.argv[2]))
def walk(s):
 for t in s:
  if isinstance(t,unittest.TestSuite): yield from walk(t)
  else: yield t.id()
print(json.dumps(sorted(set(walk(suite)))))
"""
    with tempfile.TemporaryDirectory(prefix="huey-inventory-") as temporary:
        result = subprocess.run(
            [
                sys.executable,
                "-I",
                "-B",
                "-S",
                "-c",
                collect,
                str(repo.resolve()),
                json.dumps(config["modules"]),
            ],
            cwd=temporary,
            capture_output=True,
            text=True,
            check=True,
            env={
                "PATH": "/usr/bin:/bin",
                "HOME": temporary,
                "PYTHONDONTWRITEBYTECODE": "1",
                "HUEY_SLOW_TESTS": "1",
            },
        )
    ids = json.loads(result.stdout)
    if not ids or any("_FailedTest" in item for item in ids):
        raise ValueError(
            "Pinned upstream regression inventory did not load successfully."
        )
    exclusions = config.get("exclude_tests", {})
    if any(item not in ids for item in exclusions):
        raise ValueError(
            "A declared regression exclusion does not match the pinned inventory."
        )
    ids = [item for item in ids if item not in exclusions]
    contract = list(FrozenTests.read(output).expected)
    isolated = [
        item
        for item in ids
        if item.startswith("huey.tests.test_storage.TestSqliteStorage.")
    ]
    groups = [[item for item in ids if item not in isolated] + contract] + [
        [item] for item in isolated
    ]
    plan: Inventory = {
        "upstream_tests": ids,
        "contract_tests": contract,
        "groups": groups,
        "environment": {"HUEY_SLOW_TESTS": "1"},
        "scope": config["scope"],
        "source": source,
        "excluded_tests": exclusions,
        "sqlite_case_isolation": "Each unmodified upstream SQLite case uses a new process and working directory to avoid open handles to deleted WAL database files.",
    }
    (output / "inventory.json").write_text(json.dumps(plan, indent=2) + "\n")
    return plan


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--definition", type=Path, required=True)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    plan = prepare(
        args.definition.resolve(), args.archive.resolve(), args.output.resolve()
    )
    print(
        json.dumps(
            {
                "upstream_tests": len(plan["upstream_tests"]),
                "contract_tests": len(plan["contract_tests"]),
                "process_groups": len(plan["groups"]),
            }
        )
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
