"""Grade the pinned Huey repository task against frozen regression identities."""

from __future__ import annotations

import json
import math
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from bench.lib.run import stop_process_group
from bench.lib.test_receipts import FrozenTests, check_receipt, result_path, save_result


def _profile_path(path: Path) -> str:
    return json.dumps(str(path.resolve()))


def _delivery_artifacts(fixture: Path, work: Path) -> dict[str, list[str]]:
    changed_tests = []
    changed_docs = []
    for base in (work / "tests", work / "huey/tests"):
        for path in base.rglob("test_*.py"):
            original = fixture / "repo" / path.relative_to(work)
            if path.read_bytes().strip() and (
                not original.exists() or path.read_bytes() != original.read_bytes()
            ):
                changed_tests.append(str(path.relative_to(work)))
    for path in [
        work / "README.rst",
        *(work / "docs").rglob("*.rst"),
        *(work / "docs").rglob("*.md"),
    ]:
        if path.is_file():
            original = fixture / "repo" / path.relative_to(work)
            if path.read_bytes().strip() and (
                not original.exists() or path.read_bytes() != original.read_bytes()
            ):
                changed_docs.append(str(path.relative_to(work)))
    return {
        "test_files": sorted(set(changed_tests)),
        "documentation_files": sorted(set(changed_docs)),
    }


def grade(fixture: Path, work: Path, output: Path, timeout: float) -> int:
    if not math.isfinite(timeout) or timeout <= 0:
        raise ValueError("Repository grading requires a positive finite total timeout.")
    output.mkdir(parents=True, exist_ok=False)
    plan = json.loads((fixture / "inventory.json").read_text())
    groups = plan["groups"]
    flat = [item for group in groups for item in group]
    if not flat or len(flat) != len(set(flat)):
        raise ValueError(
            "Frozen repository inventory must contain distinct nonempty identities."
        )
    deadline = time.monotonic() + timeout
    results = []
    with tempfile.TemporaryDirectory(prefix="bruhs-repository-grade-") as temporary:
        directory = Path(temporary)
        target = directory / "work"
        shutil.copytree(
            work,
            target,
            symlinks=True,
            ignore=shutil.ignore_patterns(".git", "__pycache__", "_bench_hidden"),
        )
        if any(path.is_symlink() for path in target.rglob("*")):
            (output / "report.json").write_text(
                json.dumps(
                    {
                        "passed": False,
                        "expected_tests": len(flat),
                        "groups": [],
                        "invalid_submission": "Generated repository contains a symlink; grading refused.",
                    },
                    indent=2,
                )
                + "\n"
            )
            return 1
        artifacts = _delivery_artifacts(fixture, target)
        # Restore upstream test bytes before importing any submitted application.
        test_path = target / "huey/tests"
        if test_path.is_dir():
            shutil.rmtree(test_path)
        elif test_path.exists():
            test_path.unlink()
        shutil.copytree(fixture / "repo/huey/tests", target / "huey/tests")
        FrozenTests.read(fixture).install(target)
        runner = directory / "test_runner.py"
        shutil.copy2(Path(__file__).resolve().parents[1] / "lib/test_runner.py", runner)
        case_root = Path(tempfile.mkdtemp(prefix=".grading-", dir=target))
        for index, names in enumerate(groups):
            cwd = case_root / str(index)
            cwd.mkdir(parents=True)
            log = output / f"group-{index}.log"
            channel_path = directory / f"receipt-{index}.json"
            profile = "\n".join(
                [
                    "(version 1)",
                    "(allow default)",
                    "(deny network*)",
                    f"(deny file-write* (require-all (require-not (subpath {_profile_path(target)})) (require-not (literal {_profile_path(channel_path)}))))",
                    f"(deny file-read* (require-all (subpath {_profile_path(Path.home())}) (require-not (subpath {_profile_path(Path(sys.base_prefix))})) (require-not (subpath {_profile_path(target)})) (require-not (literal {_profile_path(runner)}))))",
                ]
            )
            status = 124
            raw = b""
            remaining = deadline - time.monotonic()
            with log.open("xb") as stream:
                if remaining > 0:
                    with channel_path.open("x+b") as channel:
                        command = [
                            "/usr/bin/sandbox-exec",
                            "-p",
                            profile,
                            sys.executable,
                            "-I",
                            "-S",
                            str(runner),
                            str(channel.fileno()),
                            "--root",
                            str(target),
                        ]
                        for name in names:
                            command.extend(["--test", name])
                        process = subprocess.Popen(
                            command,
                            cwd=cwd,
                            stdin=subprocess.DEVNULL,
                            stdout=stream,
                            stderr=subprocess.STDOUT,
                            env={
                                "PATH": "/usr/bin:/bin",
                                "HOME": str(target),
                                "PYTHONDONTWRITEBYTECODE": "1",
                                **plan["environment"],
                            },
                            pass_fds=(channel.fileno(),),
                            start_new_session=True,
                        )
                        try:
                            status = process.wait(timeout=remaining)
                        except subprocess.TimeoutExpired:
                            stop_process_group(process)
                        except BaseException:
                            stop_process_group(process)
                            raise
                        channel.seek(0)
                        raw = channel.read()
            checked = check_receipt(status, raw, tuple(sorted(names)))
            save_result(log, raw, checked)
            results.append(
                {
                    "group": index,
                    "passed": checked.passed,
                    "process_exit": status,
                    "expected_tests": len(names),
                    "result": str(result_path(log)),
                    "errors": list(checked.errors),
                }
            )
    artifact_errors = []
    if not artifacts["test_files"]:
        artifact_errors.append("No added or changed test file was supplied.")
    if not artifacts["documentation_files"]:
        artifact_errors.append("No added or changed documentation was supplied.")
    report = {
        "passed": all(row["passed"] for row in results) and not artifact_errors,
        "expected_tests": len(flat),
        "groups": results,
        "delivery_artifacts": artifacts,
        "artifact_errors": artifact_errors,
        "review_required": "Inspect the submitted diff for relevant tests/documentation, API scope and regressions beyond this selected suite.",
        "scope": "Frozen identities and pristine regressions; shared-interpreter receipts are not adversarial attestation.",
    }
    (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    return 0 if report["passed"] else 1
