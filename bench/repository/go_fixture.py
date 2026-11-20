"""Prepare and grade the pinned, dependency-free Go scheduler fixture."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
from collections import Counter
from pathlib import Path
from typing import TypedDict

from bench.lib.run import stop_process_group


class GoInventory(TypedDict):
    language: str
    package: str
    upstream_tests: list[str]
    public_tests: list[str]
    contract_tests: list[str]
    go_binary: str
    go_sha256: str
    source: dict


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def test_names(paths: list[Path]) -> list[str]:
    names = [
        name
        for path in paths
        for name in re.findall(
            r"^func\s+(Test\w+)\s*\(", path.read_text(), re.MULTILINE
        )
    ]
    if not names or "TestMain" in names or len(names) != len(set(names)):
        raise ValueError(
            "Go fixture needs distinct literal top-level tests and no TestMain."
        )
    return sorted(names)


def provider_wrapper(go: Path, path: Path) -> None:
    """Provide an offline Go wrapper with per-workspace caches."""
    source = f"""#!{sys.executable}
import os, pathlib, sys
state = pathlib.Path.cwd() / ".bench-go"
(state / "tmp").mkdir(parents=True, exist_ok=True)
go_home = state / "home"
mode = go_home / "Library/Application Support/go/telemetry/mode"
mode.parent.mkdir(parents=True, exist_ok=True)
mode.write_text("off\\n")
os.environ.update({{"GOENV":"off", "GOTOOLCHAIN":"local", "HOME":str(go_home),
    "GOWORK":"off", "GOPROXY":"off", "GOSUMDB":"off", "CGO_ENABLED":"1",
    "GOCACHE":str(state / "cache"), "GOPATH":str(state / "path"),
    "TMPDIR":str(state / "tmp"), "GOFLAGS":"-buildvcs=false"}})
os.execv({str(go)!r}, [{str(go)!r}, *sys.argv[1:]])
"""
    path.write_text(source)
    path.chmod(0o700)


def prepare(definition: Path, archive: Path, output: Path, go: Path) -> GoInventory:
    source = json.loads((definition / "source.json").read_text())
    if digest(archive) != source["archive_sha256"]:
        raise ValueError("Go source archive does not match its pinned checksum.")
    output.mkdir(parents=True, exist_ok=False)
    with tarfile.open(archive) as package:
        for member in package.getmembers():
            if (
                not (member.isfile() or member.isdir())
                or Path(member.name).is_absolute()
                or ".." in Path(member.name).parts
            ):
                raise ValueError(
                    "Go source archive contains an unsupported path or link."
                )
        package.extractall(output / "unpack", filter="data")
    unpack = output / "unpack"
    roots = list(unpack.iterdir())
    if len(roots) != 1 or not roots[0].is_dir():
        raise ValueError("Go archive must contain one repository root.")
    roots[0].rename(output / "repo")
    unpack.rmdir()
    repo = output / "repo"
    upstream = test_names(sorted(repo.glob("*_test.go")))
    shutil.copy2(definition / "visible_test.go", repo / "pause_smoke_test.go")
    shutil.copy2(definition / "prompt.md", repo / "PAUSE_RESUME_SPEC.md")
    with (repo / ".gitignore").open("a") as stream:
        stream.write("\n# Private, per-attempt Go compilation state.\n.bench-go/\n")
    shutil.copytree(definition / "hidden", output / "hidden")
    for name in ("source.json", "prompt.md", "reference.diff", "campaign.json"):
        shutil.copy2(definition / name, output / name)
    public = test_names([repo / "pause_smoke_test.go"])
    contract = test_names(sorted((output / "hidden").glob("*_test.go")))
    all_names = upstream + public + contract
    if len(all_names) != len(set(all_names)):
        raise ValueError(
            "Go regression, public and contract test names must be distinct."
        )
    plan: GoInventory = {
        "language": "go",
        "package": "github.com/robfig/cron/v3",
        "upstream_tests": upstream,
        "public_tests": public,
        "contract_tests": contract,
        "go_binary": str(go.resolve()),
        "go_sha256": digest(go.resolve()),
        "source": source,
    }
    (output / "inventory.json").write_text(json.dumps(plan, indent=2) + "\n")
    return plan


def check_events(status: int, raw: str, expected: list[str], package: str) -> dict:
    """Require process, package and every frozen top-level test to finish."""
    errors: list[str] = []
    started: Counter[str] = Counter()
    passed: Counter[str] = Counter()
    packages: Counter[str] = Counter()
    child_started: Counter[str] = Counter()
    child_passed: Counter[str] = Counter()
    children: set[str] = set()
    if status != 0:
        errors.append(f"Go test process exited {status}.")
    if not expected or len(expected) != len(set(expected)):
        errors.append("Frozen Go inventory is empty or duplicated.")
    for line in raw.splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            errors.append("Go test output contains a non-JSON event.")
            continue
        if not isinstance(event, dict):
            errors.append("Go test event is not an object.")
            continue
        action, name = event.get("Action"), event.get("Test")
        if action in ("fail", "skip"):
            errors.append(f"Go {action}: {name or event.get('Package')}.")
        if action in ("run", "pass") and name:
            if event.get("Package") != package or not isinstance(name, str):
                errors.append("Go test identity has an unexpected package or type.")
            elif "/" in name:
                children.add(name)
                (child_started if action == "run" else child_passed)[name] += 1
                if name.split("/", 1)[0] not in expected:
                    errors.append("Go subtest has no declared parent test.")
            elif action == "run":
                started[name] += 1
            else:
                passed[name] += 1
        elif action == "pass" and not name:
            packages[event.get("Package")] += 1
    wanted = Counter(expected)
    if started != wanted:
        errors.append("Started Go tests do not match the complete frozen inventory.")
    if passed != wanted:
        errors.append("Passed Go tests do not match the complete frozen inventory.")
    if packages != Counter({package: 1}):
        errors.append("The expected Go package did not finish exactly once.")
    if child_started != child_passed or any(
        count != 1 for count in child_started.values()
    ):
        errors.append("Observed Go subtests did not each start and pass exactly once.")
    return {
        "passed": not errors,
        "process_exit": status,
        "expected_top_level_tests": len(expected),
        "started": dict(sorted(started.items())),
        "successful": dict(sorted(passed.items())),
        "observed_subtests": sorted(children),
        "errors": errors,
    }


def grade(fixture: Path, work: Path, output: Path, timeout: float) -> int:
    if not math.isfinite(timeout) or timeout <= 0:
        raise ValueError("Go grading requires a positive finite total timeout.")
    output.mkdir(parents=True, exist_ok=False)
    plan = json.loads((fixture / "inventory.json").read_text())
    go = Path(plan["go_binary"])
    if digest(go) != plan["go_sha256"]:
        raise ValueError("The pinned Go compiler changed after fixture preparation.")
    expected = plan["upstream_tests"] + plan["public_tests"] + plan["contract_tests"]
    with tempfile.TemporaryDirectory(prefix="bruhs-go-grade-") as temporary:
        target = Path(temporary).resolve() / "work"
        shutil.copytree(
            work,
            target,
            symlinks=True,
            ignore=shutil.ignore_patterns(".git", ".bench-go", "__pycache__"),
        )
        invalid = ""
        if any(p.is_symlink() for p in target.rglob("*")):
            invalid = "Generated Go repository contains a symlink; grading refused."
        if not invalid and (
            not (target / "go.mod").is_file()
            or (target / "go.mod").read_bytes()
            != (fixture / "repo/go.mod").read_bytes()
        ):
            invalid = "The dependency-free module contract was changed."
        tests = (
            [
                str(p.relative_to(target))
                for p in target.rglob("*_test.go")
                if p.read_bytes().strip()
                and (
                    not (fixture / "repo" / p.relative_to(target)).is_file()
                    or p.read_bytes()
                    != (fixture / "repo" / p.relative_to(target)).read_bytes()
                )
            ]
            if not invalid
            else []
        )
        documentation_paths = sorted({*target.rglob("*.md"), target / "doc.go"})
        docs = (
            [
                str(p.relative_to(target))
                for p in documentation_paths
                if p.is_file()
                and p.name != "PAUSE_RESUME_SPEC.md"
                and p.read_bytes().strip()
                and (
                    not (fixture / "repo" / p.relative_to(target)).is_file()
                    or p.read_bytes()
                    != (fixture / "repo" / p.relative_to(target)).read_bytes()
                )
            ]
            if not invalid
            else []
        )
        if not invalid:
            # Entrant tests are reviewed separately. Restore every pinned regression.
            for p in target.rglob("*_test.go"):
                p.unlink()
            for p in (fixture / "repo").glob("*_test.go"):
                shutil.copy2(p, target / p.name)
            for p in (fixture / "hidden").glob("*_test.go"):
                shutil.copy2(p, target / p.name)
            state = target / ".bench-go"
            (state / "tmp").mkdir(parents=True)
            go_home = state / "home"
            mode = go_home / "Library/Application Support/go/telemetry/mode"
            mode.parent.mkdir(parents=True)
            mode.write_text("off\n")
            profile = "\n".join(
                [
                    "(version 1)",
                    "(allow default)",
                    "(deny network*)",
                    f"(deny file-write* (require-not (subpath {json.dumps(str(target))})))",
                    f"(deny file-read* (require-all (subpath {json.dumps(str(Path.home()))}) (require-not (subpath {json.dumps(str(target))}))))",
                ]
            )
            env = {
                "PATH": "/usr/bin:/bin",
                "HOME": str(go_home),
                "GOENV": "off",
                "GOTOOLCHAIN": "local",
                "GOWORK": "off",
                "GOPROXY": "off",
                "GOSUMDB": "off",
                "CGO_ENABLED": "1",
                "GOCACHE": str(state / "cache"),
                "GOPATH": str(state / "path"),
                "TMPDIR": str(state / "tmp"),
                "GOFLAGS": "-buildvcs=false",
            }
            command = [
                "/usr/bin/sandbox-exec",
                "-p",
                profile,
                str(go),
                "test",
                "-json",
                "-race",
                "-count=1",
                f"-timeout={timeout}s",
                ".",
            ]
            status = 124
            with (
                (output / "tests.jsonl").open("xb") as out,
                (output / "tests.err").open("xb") as err,
            ):
                process = subprocess.Popen(
                    command,
                    cwd=target,
                    env=env,
                    stdin=subprocess.DEVNULL,
                    stdout=out,
                    stderr=err,
                    start_new_session=True,
                )
                try:
                    status = process.wait(timeout=timeout)
                except subprocess.TimeoutExpired:
                    stop_process_group(process)
                except (OSError, KeyboardInterrupt, SystemExit):
                    stop_process_group(process)
                    raise
            result = check_events(
                status, (output / "tests.jsonl").read_text(), expected, plan["package"]
            )
        else:
            result = {
                "passed": False,
                "errors": [invalid],
                "expected_top_level_tests": len(expected),
            }
    artifacts = []
    if not tests:
        artifacts.append("No added or changed test file was supplied.")
    if not docs:
        artifacts.append("No added or changed documentation was supplied.")
    report = {
        **result,
        "passed": result["passed"] and not artifacts,
        "delivery_artifacts": {
            "test_files": sorted(tests),
            "documentation_files": sorted(docs),
        },
        "artifact_errors": artifacts,
        "race_detector": True,
        "scope": "All frozen top-level identities and observed subtests, pristine upstream tests, no network or home reads. Go JSON events are not adversarial attestation.",
        "review_required": "Inspect tests, documentation, API compatibility and concurrency behavior; test success is provisional.",
    }
    (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    return 0 if report["passed"] else 1


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--definition", type=Path, required=True)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--go", type=Path, default=shutil.which("go"))
    args = parser.parse_args()
    if args.go is None:
        parser.error("An installed Go compiler is required.")
    plan = prepare(
        args.definition.resolve(),
        args.archive.resolve(),
        args.output.resolve(),
        Path(args.go),
    )
    print(
        json.dumps(
            {
                k: len(plan[k])
                for k in ("upstream_tests", "public_tests", "contract_tests")
            }
        )
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
