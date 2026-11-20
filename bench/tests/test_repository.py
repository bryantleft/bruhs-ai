"""Repository campaign declarations and frozen runner entry points; no models."""

from __future__ import annotations

import copy
import hashlib
import json
import os
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

from bench.lib.test_receipts import check_receipt
from bench.live.run import capture_patch, provider_wrapper
from bench.repository.prepare import prepare
from bench.repository.run import audit_runtime, freeze, validate_campaign
from bench.workbench.run import Campaign

ROOT = Path(__file__).resolve().parents[2]


class RepositoryCampaignTests(unittest.TestCase):
    def test_declared_cells_and_unique_attempt_paths_are_required(self) -> None:
        plan = json.loads(
            (ROOT / "bench/repository/huey-leases/campaign.json").read_text()
        )
        validate_campaign(plan)
        for mutation in (
            "duplicate",
            "missing",
            "unsafe-label",
            "reused-label",
            "bad-timeout",
            "bad-config",
            "bad-daemon",
            "unsafe-task",
            "bad-task-type",
            "bad-language",
        ):
            with self.subTest(case=mutation):
                invalid = copy.deepcopy(plan)
                if mutation == "duplicate":
                    invalid["order"].append(invalid["order"][0])
                elif mutation == "missing":
                    invalid["order"].pop()
                elif mutation == "unsafe-label":
                    invalid["order"][0]["attempt_label"] = "../outside"
                elif mutation == "reused-label":
                    invalid["order"][3]["attempt_label"] = invalid["order"][0][
                        "attempt_label"
                    ]
                elif mutation == "bad-config":
                    invalid["provider_config"] = {"features.plugins": "false"}
                elif mutation == "bad-daemon":
                    invalid["native_cli_no_daemon"] = "false"
                elif mutation == "unsafe-task":
                    invalid["task_id"] = "../outside"
                elif mutation == "bad-task-type":
                    invalid["task_id"] = 42
                elif mutation == "bad-language":
                    invalid["language"] = "unsupported"
                else:
                    invalid["timeout_seconds_per_provider_and_grading_step"] = True
                with self.assertRaises(ValueError):
                    validate_campaign(invalid)

    def test_provider_wrapper_pins_catalog_policy_and_daemon_mode(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            native = root / "native"
            native.write_text(
                f"#!{sys.executable}\nimport json,sys\nprint(json.dumps(sys.argv[1:]))\n"
            )
            native.chmod(0o700)
            wrapper = root / "codex"
            calls = root / "calls.jsonl"
            settings = {
                "features.remote_plugin": False,
                "skills.include_instructions": False,
            }
            provider_wrapper(native, wrapper, calls, "low", settings, True)
            result = subprocess.run(
                [
                    str(wrapper),
                    "--ask-for-approval=never",
                    "exec",
                    "--sandbox=workspace-write",
                    "READY",
                ],
                capture_output=True,
                text=True,
                check=True,
            )
            args = json.loads(result.stdout)
            self.assertIn("--no-daemon", args)
            self.assertIn("--config=features.remote_plugin=false", args)
            self.assertIn("--config=skills.include_instructions=false", args)
            self.assertIn("--ignore-user-config", args)
            self.assertEqual(
                json.loads(calls.read_text())["config_overrides"], settings
            )

    def test_runtime_audit_rejects_injected_catalog_and_missing_rollout(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            work = root / "work"
            sessions = root / "sessions"
            sessions.mkdir()
            self.assertFalse(audit_runtime(root, work)["passed"])
            rows = [
                {"type": "session_meta", "payload": {"cwd": str(work)}},
                {
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "developer",
                        "content": [
                            {"type": "input_text", "text": "Stable native instructions"}
                        ],
                    },
                },
            ]
            path = sessions / "rollout.jsonl"
            path.write_text("\n".join(json.dumps(row) for row in rows) + "\n")
            self.assertTrue(audit_runtime(root, work)["passed"])
            rows.append(
                {
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "developer",
                        "content": [
                            {
                                "type": "input_text",
                                "text": "<skills_instructions>\n## Skills\n",
                            }
                        ],
                    },
                }
            )
            path.write_text("\n".join(json.dumps(row) for row in rows) + "\n")
            self.assertFalse(audit_runtime(root, work)["passed"])

    def test_freeze_omits_generated_caches_from_both_snapshot_and_manifest(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "source"
            source.mkdir()
            (source / "Cargo.lock").write_text("fixture lock\n")
            fixture = root / "fixture"
            (fixture / "repo/__pycache__").mkdir(parents=True)
            (fixture / "repo/example.py").write_text("pass\n")
            (fixture / "repo/__pycache__/example.pyc").write_bytes(b"generated")
            campaign = Mock(spec=Campaign)
            campaign.output = root / "evidence"
            with patch("bench.repository.run.ROOT", source):
                hashes = freeze(campaign, fixture)
            self.assertTrue(hashes)
            self.assertFalse(any("__pycache__" in key for key in hashes))
            self.assertFalse(list(campaign.output.rglob("*.pyc")))
            (fixture / "campaign.json").write_text(json.dumps({"task_id": "cron-pause"}))
            campaign.output = root / "go-evidence"
            with patch("bench.repository.run.ROOT", source):
                go_hashes = freeze(campaign, fixture)
            self.assertIn("bench/tasks/cron-pause/repo/example.py", go_hashes)
            self.assertFalse(any("tasks/huey-leases" in key for key in go_hashes))

    def test_qualified_frozen_tests_run_from_an_isolated_working_directory(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            project = root / "project"
            package = project / "fixture"
            package.mkdir(parents=True)
            (package / "__init__.py").write_text("")
            (package / "checks.py").write_text(
                "import unittest\nclass Contract(unittest.TestCase):\n"
                "    def test_runs(self): self.assertEqual(2 + 2, 4)\n"
            )
            scratch = root / "scratch"
            scratch.mkdir()
            identity = "fixture.checks.Contract.test_runs"
            with tempfile.TemporaryFile() as channel:
                result = subprocess.run(
                    [
                        sys.executable,
                        "-I",
                        "-S",
                        str(ROOT / "bench/lib/test_runner.py"),
                        str(channel.fileno()),
                        "--root",
                        str(project),
                        "--test",
                        identity,
                    ],
                    cwd=scratch,
                    pass_fds=(channel.fileno(),),
                    capture_output=True,
                    timeout=60,
                )
                channel.seek(0)
                receipt = check_receipt(result.returncode, channel.read(), (identity,))
            self.assertTrue(receipt.passed, receipt.errors)

    def test_patch_capture_keeps_new_and_committed_files_without_changing_index(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            work = root / "project"
            work.mkdir()
            env = {
                "PATH": "/usr/bin:/bin",
                "HOME": str(work),
                "GIT_CONFIG_NOSYSTEM": "1",
                "GIT_CONFIG_GLOBAL": os.devnull,
            }

            def git(*args: str) -> str:
                return subprocess.run(
                    ["git", "-C", str(work), *args],
                    env=env,
                    capture_output=True,
                    text=True,
                    check=True,
                ).stdout.strip()

            git("init", "-b", "main")
            (work / "original.py").write_text("before\n")
            git("add", ".")
            git(
                "-c",
                "user.name=fixture",
                "-c",
                "user.email=fixture@localhost",
                "commit",
                "-m",
                "start",
            )
            baseline = git("rev-parse", "HEAD")
            (work / "original.py").write_text("after\n")
            (work / "new_test.py").write_text("new test\n")
            before = (work / ".git/index").read_bytes()
            first = capture_patch(work, baseline, root / "first", env)
            self.assertIn(b"new_test.py", first)
            self.assertIn(b"+after", first)
            self.assertEqual((work / ".git/index").read_bytes(), before)
            git("add", ".")
            git(
                "-c",
                "user.name=fixture",
                "-c",
                "user.email=fixture@localhost",
                "commit",
                "-m",
                "submitted",
            )
            before = (work / ".git/index").read_bytes()
            self.assertEqual(capture_patch(work, baseline, root / "second", env), first)
            self.assertEqual((work / ".git/index").read_bytes(), before)
            relative_stem = Path(os.path.relpath(root / "relative", Path.cwd()))
            self.assertEqual(capture_patch(work, baseline, relative_stem, env), first)
            self.assertEqual((work / ".git/index").read_bytes(), before)

    def test_archive_pin_and_frozen_regression_inventory_are_required(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            upstream = root / "upstream"
            tests = upstream / "huey/tests"
            tests.mkdir(parents=True)
            (upstream / "huey/__init__.py").write_text("")
            (tests / "__init__.py").write_text("")
            (tests / "test_storage.py").write_text(
                "import unittest\nclass TestSqliteStorage(unittest.TestCase):\n    def test_old(self): pass\n"
            )
            archive = root / "upstream.tar.gz"
            with tarfile.open(archive, "w:gz") as package:
                package.add(upstream, arcname="upstream")
            definition = root / "definition"
            (definition / "hidden").mkdir(parents=True)
            (definition / "hidden/test_contract.py").write_text(
                "import unittest\nclass Contract(unittest.TestCase):\n    def test_new(self): pass\n"
            )
            (definition / "visible_tests.py").write_text(
                "import unittest\nclass Smoke(unittest.TestCase):\n    def test_public(self): pass\n"
            )
            (definition / "source.json").write_text(
                json.dumps(
                    {"archive_sha256": hashlib.sha256(archive.read_bytes()).hexdigest()}
                )
            )
            (definition / "regressions.json").write_text(
                json.dumps(
                    {
                        "modules": ["huey.tests.test_storage.TestSqliteStorage"],
                        "scope": "synthetic",
                    }
                )
            )
            for name in ("prompt.md", "reference.diff", "campaign.json"):
                (definition / name).write_text("{}")
            result = prepare(definition, archive, root / "prepared")
            self.assertEqual(len(result["upstream_tests"]), 1)
            self.assertEqual(len(result["contract_tests"]), 2)
            self.assertEqual(len(result["groups"]), 2)
            self.assertFalse(list((root / "prepared/repo").rglob("__pycache__")))
            (definition / "source.json").write_text(
                json.dumps({"archive_sha256": "wrong"})
            )
            with self.assertRaisesRegex(ValueError, "checksum"):
                prepare(definition, archive, root / "bad")
            self.assertFalse((root / "bad").exists())


if __name__ == "__main__":
    unittest.main()
