"""Actual task checkers, complete receipts and declared grading deadlines."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

from bench.lib.test_receipts import FrozenTests, result_path

ROOT = Path(__file__).resolve().parents[2]
COUNTS = {
    "bugfix-paginate": 9,
    "concurrency-keyed-jobs": 18,
    "explore-timeout": 10,
    "feature-inventory-csv": 6,
    "invariant-rename": 13,
    "longfile-fee-policy": 8,
    "multibug-ledger": 15,
    "refactor-shipping": 10,
    "rootcause-paging": 17,
    "spec-business-days": 27,
    "spec-invoice-tax": 34,
}


def run_check(
    fixture: Path, work: Path, output: Path, timeout: str = "60"
) -> subprocess.CompletedProcess[str]:
    env = dict(
        os.environ,
        BRUHS_BENCH_CHECK_OUTPUT=str(output),
        BRUHS_BENCH_CHECK_TIMEOUT=timeout,
    )
    return subprocess.run(
        [str(fixture / "check.sh"), str(work)],
        env=env,
        capture_output=True,
        text=True,
        timeout=60,
    )


class TaskGradingTests(unittest.TestCase):
    def test_all_coding_originals_fail_and_reference_fixes_complete_every_check(
        self,
    ) -> None:
        selected = {
            task.name
            for task in (ROOT / "bench/tasks").iterdir()
            if task.is_dir() and not (task / "kind").exists()
        }
        self.assertEqual(selected, set(COUNTS))
        with tempfile.TemporaryDirectory() as temporary:
            for name, count in COUNTS.items():
                fixture = ROOT / "bench/tasks" / name
                for variant in ("repo", "solution"):
                    with self.subTest(task=name, variant=variant):
                        directory = Path(temporary) / f"{name}-{variant}"
                        shutil.copytree(fixture / "repo", directory)
                        if variant == "solution":
                            shutil.copytree(
                                fixture / "solution", directory, dirs_exist_ok=True
                            )
                        output = directory.parent / f"{name}-{variant}.check"
                        result = run_check(fixture, directory, output)
                        self.assertEqual(
                            result.returncode == 0,
                            variant == "solution",
                            result.stdout + result.stderr,
                        )
                        receipt = json.loads(result_path(output).read_text())
                        self.assertEqual(receipt["passed"], variant == "solution")
                        self.assertEqual(len(receipt["expected_tests"]), count)
                        if variant == "solution":
                            self.assertEqual(receipt["receipt"]["tests_run"], count)
                            self.assertEqual(receipt["receipt"]["skipped"], 0)

    def test_actual_shell_checker_rejects_early_exit_and_skipped_checks(self) -> None:
        fixture = ROOT / "bench/tasks/rootcause-paging"
        with tempfile.TemporaryDirectory() as temporary:
            for name, source in {
                "early-exit": "import os\nos._exit(0)\n",
                "skipped-import": "import unittest\nraise unittest.SkipTest('contract skipped')\n",
                "skipped-tests": "import unittest\ndef skip(self):\n    self.skipTest('contract skipped')\nunittest.TestCase.setUp=skip\n",
            }.items():
                with self.subTest(case=name):
                    directory = Path(temporary) / name
                    shutil.copytree(fixture / "solution", directory)
                    (directory / "orders/__init__.py").write_text(source)
                    output = Path(temporary) / f"{name}.check"
                    result = run_check(fixture, directory, output)
                    self.assertEqual(
                        result.returncode, 1, result.stdout + result.stderr
                    )
                    report = json.loads(result_path(output).read_text())
                    self.assertFalse(report["passed"])
                    self.assertEqual(len(report["expected_tests"]), 17)
                    if name == "early-exit":
                        self.assertEqual(report["process_exit"], 0)
                        self.assertIsNone(report["receipt"])
                    elif name == "skipped-tests":
                        self.assertEqual(report["receipt"]["tests_run"], 17)
                        self.assertEqual(report["receipt"]["skipped"], 17)

    def test_declared_timeout_stops_the_grading_process(self) -> None:
        fixture = ROOT / "bench/tasks/rootcause-paging"
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary) / "work"
            shutil.copytree(fixture / "solution", directory)
            (directory / "orders/__init__.py").write_text(
                "import os, signal\nfrom pathlib import Path\n"
                "Path('grader.pid').write_text(str(os.getpid()))\n"
                "while True:\n    signal.pause()\n"
            )
            output = Path(temporary) / "timeout.check"
            # Reuse the existing process-group regression's one-second budget.
            result = run_check(fixture, directory, output, timeout="1")
            self.assertEqual(result.returncode, 124, result.stderr)
            self.assertEqual(
                json.loads(result_path(output).read_text())["process_exit"], 124
            )
            pid = int((directory / "grader.pid").read_text())
            state = subprocess.run(
                ["ps", "-p", str(pid), "-o", "stat="], capture_output=True, text=True
            ).stdout.strip()
            self.assertTrue(not state or state.startswith("Z"), state)

    def test_submitted_code_does_not_inherit_parent_environment_credentials(
        self,
    ) -> None:
        fixture = ROOT / "bench/tasks/rootcause-paging"
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary) / "work"
            shutil.copytree(fixture / "solution", directory)
            (directory / "orders/__init__.py").write_text(
                "import os\nassert 'GRADING_PRIVATE_SENTINEL' not in os.environ\n"
            )
            output = Path(temporary) / "environment.check"
            env = dict(
                os.environ,
                BRUHS_BENCH_CHECK_OUTPUT=str(output),
                BRUHS_BENCH_CHECK_TIMEOUT="60",
                GRADING_PRIVATE_SENTINEL="synthetic-private-value",
            )
            result = subprocess.run(
                [str(fixture / "check.sh"), str(directory)],
                env=env,
                capture_output=True,
                text=True,
                timeout=60,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_inventory_installs_the_bytes_that_were_frozen(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            fixture = Path(temporary) / "fixture"
            (fixture / "hidden").mkdir(parents=True)
            source = fixture / "hidden/test_contract.py"
            original = b"import unittest\nclass Contract(unittest.TestCase):\n    def test_value(self):\n        self.assertEqual(1, 1)\n"
            source.write_bytes(original)
            snapshot = FrozenTests.read(fixture)
            source.write_text("raise RuntimeError('changed after freezing')\n")
            work = Path(temporary) / "work"
            snapshot.install(work)
            self.assertEqual(
                (work / "_bench_hidden/test_contract.py").read_bytes(), original
            )
            self.assertEqual(
                snapshot.expected, ("_bench_hidden.test_contract.Contract.test_value",)
            )


if __name__ == "__main__":
    unittest.main()
