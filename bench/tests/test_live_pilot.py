"""Exercise the independent grader and sandbox boundaries without model calls."""

from __future__ import annotations

import importlib.util
import json
import platform
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("live_pilot", ROOT / "bench/live/run.py")
assert SPEC is not None and SPEC.loader is not None
PILOT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PILOT)


@unittest.skipUnless(platform.system() == "Darwin", "pilot uses macOS Seatbelt")
class LivePilotTests(unittest.TestCase):
    def test_live_driver_cli_imports_in_a_fresh_process(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            result = subprocess.run(
                [sys.executable, str(ROOT / "bench/live/run.py"), "--help"],
                cwd=temporary, capture_output=True, text=True, timeout=60,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("--server", result.stdout)

    def test_independent_checks_reject_originals_and_accept_reference_fixes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            for task in PILOT.TASKS:
                fixture = ROOT / "bench/tasks" / task
                for variant in ("repo", "solution"):
                    with self.subTest(task=task, variant=variant):
                        log = Path(temporary) / f"{task}-{variant}.log"
                        code = PILOT.grade(fixture, fixture / variant, log, 60)
                        self.assertEqual(code == 0, variant == "solution", log.read_text())
                        recorded = json.loads(PILOT.result_path(log).read_text())
                        self.assertEqual(recorded["passed"], variant == "solution")
                        if variant == "solution":
                            count = {"rootcause-paging": 17, "concurrency-keyed-jobs": 18}[task]
                            self.assertEqual(len(recorded["expected_tests"]), count)
                            self.assertEqual(recorded["receipt"]["tests_run"], count)
                            self.assertEqual(recorded["receipt"]["skipped"], 0)

    def test_clean_exit_or_skipped_contract_cannot_pass(self) -> None:
        fixture = ROOT / "bench/tasks/rootcause-paging"
        mutations = {
            "early-exit": "import os\nos._exit(0)\n",
            "forged-log": "import os\nprint('Ran 17 tests\\nOK', flush=True)\nos._exit(0)\n",
            "skipped-import": "import unittest\nraise unittest.SkipTest('contract skipped')\n",
            "skipped-tests": (
                "import unittest\n"
                "def skip(self):\n    self.skipTest('contract skipped')\n"
                "unittest.TestCase.setUp = skip\n"
            ),
        }
        with tempfile.TemporaryDirectory() as temporary:
            for name, source in mutations.items():
                with self.subTest(case=name):
                    work = Path(temporary) / name
                    shutil.copytree(fixture / "solution", work)
                    (work / "orders/__init__.py").write_text(source, encoding="utf-8")
                    log = Path(temporary) / f"{name}.log"
                    self.assertEqual(PILOT.grade(fixture, work, log, 60), 1, log.read_text())
                    result = json.loads(PILOT.result_path(log).read_text())
                    self.assertFalse(result["passed"])
                    self.assertTrue(result["errors"])
                    self.assertEqual(len(result["expected_tests"]), 17)
                    if name in ("early-exit", "forged-log"):
                        self.assertEqual(result["process_exit"], 0)
                        self.assertIsNone(result["receipt"])
                    elif name == "skipped-tests":
                        self.assertEqual(result["receipt"]["tests_run"], 17)
                        self.assertEqual(result["receipt"]["skipped"], 17)

    def test_empty_frozen_suite_is_a_failed_grade(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            fixture = root / "fixture"
            (fixture / "hidden").mkdir(parents=True)
            (fixture / "repo/tests").mkdir(parents=True)
            work = root / "work"
            work.mkdir()
            log = root / "grade.log"
            self.assertEqual(PILOT.grade(fixture, work, log, 60), 1)
            result = json.loads(PILOT.result_path(log).read_text())
            self.assertIsNone(result["process_exit"])
            self.assertFalse(result["passed"])
            self.assertIn("no independent tests", result["errors"][0])

    def test_grading_keeps_previous_artifacts(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            log = Path(temporary) / "grade.log"
            log.write_text("Retained previous attempt", encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "fresh output path"):
                fixture = ROOT / "bench/tasks/rootcause-paging"
                PILOT.grade(fixture, fixture / "solution", log, 60)
            self.assertEqual(log.read_text(), "Retained previous attempt")

    def test_visible_test_cannot_overwrite_a_hidden_test(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            fixture = Path(temporary) / "fixture"
            shutil.copytree(ROOT / "bench/tasks/rootcause-paging", fixture)
            shutil.copy2(fixture / "repo/tests/test_api.py", fixture / "hidden/test_api.py")
            with self.assertRaisesRegex(ValueError, "filenames collide"):
                PILOT.grade(fixture, fixture / "solution", Path(temporary) / "grade.log", 60)

    def test_grading_refuses_generated_symlinks(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            work = Path(temporary) / "work"
            work.mkdir()
            (work / "escape").symlink_to(Path.home())
            with self.assertRaisesRegex(ValueError, "symlink"):
                PILOT.grade(ROOT / "bench/tasks/rootcause-paging", work, Path(temporary) / "log", 60)

    def test_generated_code_cannot_read_user_data_or_write_outside_grade(self) -> None:
        with tempfile.TemporaryDirectory(dir=Path.home(), prefix=".bruhs-sandbox-probe-") as temporary:
            root = Path(temporary)
            secret = root / "secret.txt"
            secret.write_text("synthetic private sentinel", encoding="utf-8")
            escaped = root / "escaped.txt"
            work = root / "work"
            shutil.copytree(ROOT / "bench/tasks/rootcause-paging/solution", work)
            # Importing generated code actually attempts both operations.
            (work / "orders/__init__.py").write_text(
                "from pathlib import Path\n"
                f"for operation in (lambda: Path({str(secret)!r}).read_text(), "
                f"lambda: Path({str(escaped)!r}).write_text('escaped')):\n"
                "    try:\n        operation()\n"
                "    except PermissionError:\n        pass\n"
                "    else:\n        raise RuntimeError('sandbox allowed access')\n", encoding="utf-8")
            log = root / "check.log"
            self.assertEqual(PILOT.grade(ROOT / "bench/tasks/rootcause-paging", work, log, 60), 0, log.read_text())
            self.assertFalse(escaped.exists())

    def test_provider_wrapper_rejects_missing_or_unsafe_sandbox_policy(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            wrapper = root / "codex"
            PILOT.provider_wrapper(Path("/usr/bin/true"), wrapper, root / "calls", "low")
            for args in (["exec"], ["exec", "--dangerously-bypass-approvals-and-sandbox"]):
                with self.subTest(args=args):
                    result = subprocess.run([str(wrapper), *args], capture_output=True, text=True, timeout=60)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertFalse((root / "calls").exists())
            result = subprocess.run([str(wrapper), "--ask-for-approval=never", "exec",
                                     "--sandbox=workspace-write"], capture_output=True, text=True, timeout=60)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads((root / "calls").read_text())["effort"], "low")


if __name__ == "__main__":
    unittest.main()
