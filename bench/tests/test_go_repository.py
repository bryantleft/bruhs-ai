"""Go completion accounting and isolated repository input controls; no models."""

from __future__ import annotations

import hashlib
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from bench.repository.go_fixture import check_events, grade, provider_wrapper
from bench.repository.run import audit_runtime

PACKAGE = "fixture.example/scheduler"


def event(action: str, name: str | None = None, package: str = PACKAGE) -> dict:
    row = {"Action": action, "Package": package}
    if name is not None:
        row["Test"] = name
    return row


class GoRepositoryTests(unittest.TestCase):
    def test_declared_model_effort_and_native_sandbox_are_observed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            work = root / "work"
            (root / "sessions").mkdir()
            path = root / "sessions/rollout.jsonl"
            rows = [{"type": "session_meta", "payload": {"cwd": str(work)}}]
            path.write_text("\n".join(map(json.dumps, rows)))
            self.assertFalse(audit_runtime(root, work, "controlled", "high")["passed"])
            context = {
                "model": "controlled",
                "effort": "high",
                "approval_policy": "never",
                "sandbox_policy": {"type": "workspace-write", "network_access": False},
            }
            rows.append({"type": "turn_context", "payload": context})
            path.write_text("\n".join(map(json.dumps, rows)))
            self.assertTrue(audit_runtime(root, work, "controlled", "high")["passed"])
            self.assertFalse(audit_runtime(root, work, "other", "high")["passed"])
            self.assertFalse(audit_runtime(root, work, "controlled", "low")["passed"])
            context["sandbox_policy"]["network_access"] = True
            path.write_text("\n".join(map(json.dumps, rows)))
            self.assertFalse(audit_runtime(root, work, "controlled", "high")["passed"])

    def test_complete_top_level_and_nested_test_events_pass(self) -> None:
        rows = [
            event("run", "TestControl"),
            event("run", "TestControl/nested"),
            event("pass", "TestControl/nested"),
            event("pass", "TestControl"),
            event("pass"),
        ]
        result = check_events(
            0, "\n".join(map(json.dumps, rows)), ["TestControl"], PACKAGE
        )
        self.assertTrue(result["passed"], result)
        self.assertEqual(result["observed_subtests"], ["TestControl/nested"])

    def test_missing_duplicate_skipped_failed_or_foreign_events_cannot_pass(
        self,
    ) -> None:
        complete = [
            event("run", "TestControl"),
            event("pass", "TestControl"),
            event("pass"),
        ]
        cases = {
            "empty": [],
            "early-exit": complete[:1],
            "no-package": complete[:2],
            "duplicate": complete + [event("pass", "TestControl")],
            "unexpected": complete
            + [event("run", "TestOther"), event("pass", "TestOther")],
            "skip-child": complete + [event("skip", "TestControl/skipped")],
            "fail-child": complete + [event("fail", "TestControl/failed")],
            "unfinished-child": complete + [event("run", "TestControl/unfinished")],
            "orphan-child": complete
            + [event("run", "TestOther/child"), event("pass", "TestOther/child")],
            "foreign": [event("run", "TestControl", "other"), *complete[1:]],
            "double-package": complete + [event("pass")],
        }
        for name, rows in cases.items():
            with self.subTest(case=name):
                result = check_events(
                    0, "\n".join(map(json.dumps, rows)), ["TestControl"], PACKAGE
                )
                self.assertFalse(result["passed"], result)
        valid = "\n".join(map(json.dumps, complete))
        for status, raw, inventory in [
            (1, valid, ["TestControl"]),
            (0, valid + "\nnot json", ["TestControl"]),
            (0, valid, []),
            (0, valid, ["TestControl", "TestControl"]),
        ]:
            with self.subTest(status=status, inventory=inventory, raw=raw):
                self.assertFalse(
                    check_events(status, raw, inventory, PACKAGE)["passed"]
                )

    def test_go_wrapper_is_offline_and_uses_workspace_local_caches(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            binary = root / "native"
            binary.write_text(
                f"#!{sys.executable}\nimport json,os,sys\nprint(json.dumps({{'args':sys.argv[1:],'cache':os.environ['GOCACHE'],'proxy':os.environ['GOPROXY'],'env':os.environ['GOENV'],'toolchain':os.environ['GOTOOLCHAIN']}}))\n"
            )
            binary.chmod(0o700)
            wrapper = root / "go"
            provider_wrapper(binary, wrapper)
            result = subprocess.run(
                [str(wrapper), "test", "-race", "."],
                cwd=root,
                capture_output=True,
                text=True,
                check=True,
            )
            data = json.loads(result.stdout)
            self.assertEqual(data["args"], ["test", "-race", "."])
            self.assertEqual(data["cache"], str(root / ".bench-go/cache"))
            self.assertEqual(
                (data["proxy"], data["env"], data["toolchain"]), ("off", "off", "local")
            )

    def test_invalid_module_or_symlink_is_a_failed_submission_without_execution(
        self,
    ) -> None:
        for kind in ("missing", "changed", "symlink"):
            with self.subTest(case=kind), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                fixture, work = root / "fixture", root / "work"
                (fixture / "repo").mkdir(parents=True)
                work.mkdir()
                module = fixture / "repo/go.mod"
                module.write_text("module fixture.example/scheduler\n\ngo 1.12\n")
                binary = Path(sys.executable).resolve()
                (fixture / "inventory.json").write_text(
                    json.dumps(
                        {
                            "go_binary": str(binary),
                            "go_sha256": hashlib.sha256(
                                binary.read_bytes()
                            ).hexdigest(),
                            "package": PACKAGE,
                            "upstream_tests": ["TestControl"],
                            "public_tests": [],
                            "contract_tests": [],
                        }
                    )
                )
                if kind == "changed":
                    (work / "go.mod").write_text("module changed\n")
                elif kind == "symlink":
                    (work / "go.mod").symlink_to(module)
                output = root / "grade"
                with patch(
                    "bench.repository.go_fixture.subprocess.Popen",
                    side_effect=AssertionError("invalid input was executed"),
                ):
                    self.assertEqual(grade(fixture, work, output, 600), 1)
                report = json.loads((output / "report.json").read_text())
                self.assertFalse(report["passed"])
                self.assertTrue(report["errors"])


if __name__ == "__main__":
    unittest.main()
