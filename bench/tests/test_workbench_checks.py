"""Reject acceptance results that hide wrong usage or provider-session reuse."""

from __future__ import annotations

import copy
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("workbench_acceptance", ROOT / "bench/workbench/run.py")
assert SPEC is not None and SPEC.loader is not None
ACCEPTANCE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ACCEPTANCE)


class UsageOracleTests(unittest.TestCase):
    def check(self, mutation: str | None = None) -> None:
        record = {"thread_id": "task", "model": "codex/fixture-model", "input": 13,
                  "cache_read": 7, "output": 3, "cache_write": 0}
        total = {"turns": 1, "unpriced_turns": 1, "input": 13, "cache_read": 7,
                 "output": 3, "cache_write": 0, "estimated_usd": None}
        tasks = [{"id": "task", "state": "completed", "messages": [{"text": "FIXTURE_CASE=success"}]}]
        usage = {"by_model": {"codex/fixture-model": copy.deepcopy(total)}, "errors": []}
        if mutation == "missing_failed_receipt":
            tasks.append({"id": "failed", "state": "failed", "messages": [{"text": "FIXTURE_CASE=failed_exit"}]})
        elif mutation == "wrong_model":
            usage["by_model"] = {"wrong-model": total}
        elif mutation is not None:
            usage["by_model"]["codex/fixture-model"][mutation] = 0
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            (path / "usage.jsonl").write_text(json.dumps({"kind": "turn", "value": record}) + "\n", encoding="utf-8")
            def api(method: str, route: str) -> dict:
                return {"tasks": tasks} if route == "/api/coordination" else usage
            ACCEPTANCE.Campaign.usage(SimpleNamespace(workbench=path, api=api))

    def test_correct_record_and_api_counts_pass(self) -> None:
        self.check()

    def test_nonempty_api_with_wrong_model_or_totals_fails(self) -> None:
        for mutation in ("wrong_model", "turns", "input", "output", "cache_read", "unpriced_turns", "estimated_usd", "missing_failed_receipt"):
            with self.subTest(mutation=mutation), self.assertRaises(AssertionError):
                self.check(mutation)


@unittest.skipUnless(os.name == "posix", "owned fixture requires POSIX")
class ProviderIdentityTests(unittest.TestCase):
    def test_fixture_honors_requested_resume_identity_in_another_workspace(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            workspace = root / "child"
            workspace.mkdir()
            result = subprocess.run([
                sys.executable, str(ROOT / "bench/workbench/fixture_codex.py"),
                f"--cd={workspace}", "resume", "--", "fixture-parent-session", "-",
            ], input="FIXTURE_CASE=success", capture_output=True, text=True, check=True,
                env={"HOME": str(root), "BRUHS_FIXTURE_LOG": str(root / "calls.jsonl")})
            events = [json.loads(line) for line in result.stdout.splitlines()]
            self.assertEqual(events[0]["thread_id"], "fixture-parent-session")
            self.assertEqual((workspace / "outcome.txt").read_text(), "fixture-parent-session")


if __name__ == "__main__":
    unittest.main()
