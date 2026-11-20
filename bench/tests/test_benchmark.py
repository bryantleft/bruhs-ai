"""Benchmark integrity checks; all harnesses and usage records are synthetic."""

from __future__ import annotations

import csv
import hashlib
import importlib
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import unittest
from collections.abc import Generator, Mapping
from contextlib import contextmanager, suppress
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "bench/lib"))
completion = importlib.import_module("completion")
metrics = importlib.import_module("metrics")
provenance = importlib.import_module("provenance")

COLUMNS = ["tool", "task", "kind", "run", "pass", "tokens", "peak_ctx", "turns",
           "tool_calls", "wall_s", "exit", "note"]


def run_row(tool: str = "codex", **changes: str) -> dict[str, str]:
    row = dict(zip(COLUMNS, [tool, "bugfix", "code", "1", "1", "100", "50", "1", "1", "2", "0", ""]))
    row.update(changes)
    return row


class ReportTests(unittest.TestCase):
    def render(self, rows: list[dict[str, str]], manifest: Mapping[str, object] | None = None) -> str:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "results.tsv"
            with path.open("w", encoding="utf-8", newline="") as handle:
                writer = csv.DictWriter(handle, COLUMNS, delimiter="\t")
                writer.writeheader()
                writer.writerows(rows)
            if manifest is not None:
                path.with_name("manifest.json").write_text(json.dumps(dict(manifest)), encoding="utf-8")
            result = subprocess.run(
                [sys.executable, str(ROOT / "bench/lib/report.py"), str(path)],
                capture_output=True, text=True, check=True,
            )
            return result.stdout

    def test_historical_timeout_with_passing_checks_is_not_a_success(self) -> None:
        report = self.render([run_row(exit="124")])
        self.assertIn("| codex | code | 0/1 |", report)
        self.assertIn("checks passed; exit 124", report)

    def test_crashed_run_with_passing_checks_is_not_a_success(self) -> None:
        report = self.render([run_row(exit="7")])
        self.assertIn("| codex | code | 0/1 |", report)

    def test_missing_exit_is_unverified(self) -> None:
        report = self.render([run_row(exit="")])
        self.assertIn("UNVERIFIED", report)
        self.assertIn("| codex | code | 0/1 |", report)

    def test_partial_usage_cannot_look_like_a_lower_cost_per_success(self) -> None:
        report = self.render([run_row(), run_row(run="2", tokens="-")])
        summary = next(line for line in report.split("## Per tool\n", 1)[1].splitlines() if line.startswith("| codex | code |"))
        self.assertIn("| 2/2 | - |", summary)
        self.assertIn(">=100 (1 unknown)", summary)
        self.assertIn("| >=100 (1 unknown) | - |", summary)

    def test_mismatched_task_coverage_is_explicit(self) -> None:
        report = self.render([run_row(), run_row("bruhs", task="another")])
        self.assertIn("## Coverage", report)
        self.assertIn("incomplete", report)
        self.assertIn("1/2", report)

    def test_selected_but_unavailable_tool_remains_in_denominator(self) -> None:
        manifest = {"tools": ["codex", "capy"], "tasks": [{"name": "bugfix", "kind": "code"}], "runs": 1}
        report = self.render([run_row()], manifest)
        self.assertIn("| capy | code | 0/1 |", report.split("## Per tool\n", 1)[1])
        self.assertIn("missing", report)

    def test_complete_usage_keeps_all_attempts_in_cost_per_success(self) -> None:
        report = self.render([run_row(), run_row(run="2", **{"pass": "0", "tokens": "200"})])
        self.assertIn("| codex | code | 1/2 | 150 | 100 | 200 | 300 | 300 |", report)

    def test_missing_wall_time_is_not_zero(self) -> None:
        report = self.render([run_row(wall_s="-")])
        self.assertIn("| 100 | 50 | - |", report)

    def test_duplicate_attempts_are_rejected(self) -> None:
        with self.assertRaises(subprocess.CalledProcessError):
            self.render([run_row(), run_row()])

    def test_unplanned_attempts_are_rejected(self) -> None:
        manifest = {"tools": ["codex"], "tasks": [{"name": "bugfix", "kind": "code"}], "runs": 1}
        with self.assertRaises(subprocess.CalledProcessError):
            self.render([run_row(run="2")], manifest)

    def test_unplanned_tools_are_rejected(self) -> None:
        manifest = {"tools": ["codex"], "tasks": [{"name": "bugfix", "kind": "code"}], "runs": 1}
        with self.assertRaises(subprocess.CalledProcessError):
            self.render([run_row("bruhs")], manifest)


class MetricsTests(unittest.TestCase):
    def test_missing_claude_usage_is_unknown_instead_of_zero(self) -> None:
        self.assertEqual(metrics.claude_metrics([{"type": "result", "is_error": False}])[0], "-")

    def test_interrupted_claude_usage_is_a_lower_bound(self) -> None:
        parsed = metrics.claude_metrics([{"type": "assistant", "message": {
            "id": "one", "usage": {"input_tokens": 10, "output_tokens": 2}}}])
        self.assertEqual(parsed[0], "-")
        self.assertIn("observed 12", parsed[4])

    def test_codex_rollout_fallback_is_not_a_complete_total(self) -> None:
        with patch.object(metrics, "codex_requests", return_value=(100, 1, 200)):
            parsed = metrics.codex_metrics([])
        self.assertEqual(parsed[0], "-")
        self.assertIn("lower bound", parsed[4])

    def test_review_total_includes_review_and_both_turns(self) -> None:
        self.assertEqual(self.review_total({"input_tokens": 20, "output_tokens": 2})[0], 46)

    def test_missing_reviewer_usage_makes_the_total_unknown(self) -> None:
        parsed = self.review_total(None)
        self.assertEqual(parsed[0], "-")
        self.assertIn("total unknown", parsed[4])

    def test_empty_or_malformed_attempted_followup_makes_the_total_unknown(self) -> None:
        for text in ("", "not-json", "{broken"):
            with self.subTest(text=text):
                parsed = self.review_total({"input_tokens": 20, "output_tokens": 2}, followup_text=text)
                self.assertEqual(parsed[0], "-")
                self.assertIn("turn 2 log empty", parsed[4])

    def test_t3_claude_missing_message_usage_is_unknown(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / "run.log"
            path.write_text(json.dumps({"status": "completed"}), encoding="utf-8")
            path.with_suffix(".cwd").write_text("fixture", encoding="utf-8")
            (root / "session.jsonl").write_text(json.dumps({"type": "assistant", "message": {"id": "one"}}), encoding="utf-8")
            with patch.object(metrics, "claude_project_dir", return_value=str(root)):
                parsed = metrics.t3_metrics("claude", str(path))
            self.assertEqual(parsed[0], "-")
            self.assertIn("missing complete usage", parsed[4])

    def test_t3_codex_rollout_without_usage_is_unknown(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / "run.log"
            path.write_text(json.dumps({"status": "completed"}), encoding="utf-8")
            path.with_suffix(".cwd").write_text("fixture", encoding="utf-8")
            rollout = root / "rollout.jsonl"
            rollout.write_text(json.dumps({"type": "session_meta"}), encoding="utf-8")
            with patch.object(metrics, "codex_rollouts_in", return_value=[(str(rollout), "one")]), \
                 patch.object(metrics, "codex_requests", return_value=(None, None, None)):
                parsed = metrics.t3_metrics("codex", str(path))
            self.assertEqual(parsed[0], "-")
            self.assertIn("missing usage", parsed[4])

    def test_codex_rollout_missing_usage_fields_is_unknown(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "rollout.jsonl"
            path.write_text(json.dumps({"payload": {"type": "token_count", "info": {
                "total_token_usage": {"total_tokens": 10}, "last_token_usage": {"input_tokens": 10}
            }}}), encoding="utf-8")
            with patch.object(metrics, "codex_rollout", return_value=str(path)):
                self.assertIsNone(metrics.codex_requests("fixture")[2])

    def review_total(self, usage: dict[str, int] | None, followup_text: str | None = None) -> tuple:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "run.log"
            event = {"envelope": "query", "type": "result", "is_error": False,
                     "usage": {"input_tokens": 10, "output_tokens": 2}}
            path.write_text(json.dumps(event), encoding="utf-8")
            path.with_suffix(".t2.log").write_text(json.dumps(event) if followup_text is None else followup_text, encoding="utf-8")
            path.with_suffix(".review.json").write_text(json.dumps({"reviewer": "codex", "review": "fix it", "usage": usage}), encoding="utf-8")
            return metrics.review_metrics("bruhs-review-claude", str(path))


class CompletionTests(unittest.TestCase):
    def test_terminal_protocols(self) -> None:
        cases = [
            ("claude", {"type": "result", "is_error": False}, "completed"),
            ("claude", {"type": "result", "is_error": True}, "failed"),
            ("codex", {"type": "turn.completed"}, "completed"),
            ("codex", {"type": "turn.failed"}, "failed"),
            ("codex", {"type": "thread.started"}, "unknown"),
            ("bruhs-claude", {"envelope": "query", "type": "error"}, "failed"),
            ("bruhs-codex", {"envelope": "query", "type": "usage"}, "completed"),
            ("t3-codex", {"status": "failed"}, "failed"),
            ("t3-claude", {"status": "completed"}, "completed"),
        ]
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "run.log"
            for fmt, event, expected in cases:
                with self.subTest(fmt=fmt, event=event):
                    path.write_text(json.dumps(event), encoding="utf-8")
                    self.assertEqual(completion.completion(fmt, str(path)), expected)

    def test_error_after_success_is_not_success(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "run.log"
            path.write_text('\n'.join(map(json.dumps, [
                {"envelope": "query", "type": "usage"},
                {"envelope": "query", "type": "error"},
            ])), encoding="utf-8")
            self.assertEqual(completion.completion("bruhs-codex", str(path)), "failed")

    def test_review_requires_successful_http_and_a_finished_followup(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "run.log"
            result = {"envelope": "query", "type": "result", "is_error": False}
            path.write_text(json.dumps(result), encoding="utf-8")
            review = path.with_suffix(".review.json")
            review.write_text(json.dumps({"review": "VERDICT: holds up"}), encoding="utf-8")
            self.assertEqual(completion.completion("bruhs-review-claude", str(path)), "unknown")
            path.with_suffix(".review.http").write_text("500", encoding="utf-8")
            self.assertEqual(completion.completion("bruhs-review-claude", str(path)), "failed")
            path.with_suffix(".review.http").write_text("200", encoding="utf-8")
            self.assertEqual(completion.completion("bruhs-review-claude", str(path)), "completed")
            review.write_text(json.dumps({"review": "Fix the bug"}), encoding="utf-8")
            self.assertEqual(completion.completion("bruhs-review-claude", str(path)), "unknown")
            path.with_suffix(".t2.log").write_text(json.dumps(result), encoding="utf-8")
            self.assertEqual(completion.completion("bruhs-review-claude", str(path)), "completed")


class SnapshotTests(unittest.TestCase):
    def test_fixtures_and_grader_are_frozen_before_a_run(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for relative in ("bench/lib/check-lib.sh", "bench/tasks/demo/check.sh",
                             "bench/tasks/demo/repo/data.txt", "scripts/task-bench.sh"):
                path = root / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("original", encoding="utf-8")
            output = root / "output"
            output.mkdir()
            provenance.snapshot(root, output, ["demo"], ["codex"], 2, {"timeout_seconds_per_step": "600"})
            manifest = json.loads((output / "manifest.json").read_text(encoding="utf-8"))
            (root / "bench/tasks/demo/check.sh").write_text("changed", encoding="utf-8")
            for relative, expected_hash in manifest["files_sha256"].items():
                frozen = output / "snapshot" / relative
                self.assertEqual(hashlib.sha256(frozen.read_bytes()).hexdigest(), expected_hash)
            self.assertEqual(manifest["runs"], 2)
            self.assertEqual(manifest["tasks"], [{"name": "demo", "kind": "code"}])

    def test_duplicate_selections_fail_before_a_paid_run(self) -> None:
        with tempfile.TemporaryDirectory() as directory, self.assertRaisesRegex(ValueError, "duplicates"):
            provenance.snapshot(Path(directory), Path(directory), ["demo", "demo"], ["codex"], 1, {})


class RunnerTests(unittest.TestCase):
    @contextmanager
    def fake_run(self, status: int = 7, terminal: str = "completed", mutate_grader: bool = False,
                 missing_receipt: bool = False) -> Generator[tuple[Path, Path], None, None]:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "scripts").mkdir()
            shutil.copy2(ROOT / "scripts/task-bench.sh", root / "scripts/task-bench.sh")
            shutil.copytree(ROOT / "bench/lib", root / "bench/lib", ignore=shutil.ignore_patterns("__pycache__"))
            task = root / "bench/tasks/bugfix-paginate"
            shutil.copytree(ROOT / "bench/tasks/bugfix-paginate", task)
            if missing_receipt:
                (task / "check.sh").write_text("#!/usr/bin/env bash\nexit 0\n", encoding="utf-8")
            bin_dir = root / "bin"
            bin_dir.mkdir()
            command = bin_dir / "codex"
            command.write_text(
                f"#!{sys.executable}\n"
                "import json, os, pathlib, shutil\n"
                "source = pathlib.Path(os.environ['BENCH_FAKE_SOLUTION'])\n"
                "shutil.copytree(source, pathlib.Path.cwd(), dirs_exist_ok=True)\n"
                "if os.environ['BENCH_FAKE_MUTATE'] == '1':\n"
                "    (source.parent / 'check.sh').write_text('#!/usr/bin/env bash\\nexit 1\\n')\n"
                "    (source.parent.parent.parent / 'lib/test_receipts.py').write_text(\"raise RuntimeError('source grader changed')\\n\")\n"
                "print(json.dumps({'type':'turn.' + os.environ['BENCH_FAKE_TERMINAL'],'usage':{'input_tokens':100,'output_tokens':1}}))\n"
                "raise SystemExit(int(os.environ['BENCH_FAKE_EXIT']))\n",
                encoding="utf-8",
            )
            command.chmod(0o755)
            env = dict(os.environ, PATH=f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
                       BENCH_TOOLS="codex", BENCH_TASKS="bugfix-paginate", BENCH_RUNS="1",
                       BENCH_CODEX_MODEL="synthetic", BENCH_WORK_ROOT=str(root / "work"),
                       BENCH_FAKE_EXIT=str(status), BENCH_FAKE_TERMINAL=terminal,
                       BENCH_FAKE_MUTATE="1" if mutate_grader else "0",
                       BENCH_FAKE_SOLUTION=str(task / "solution"))
            result = subprocess.run(
                ["bash", str(root / "scripts/task-bench.sh")], env=env,
                capture_output=True, text=True, check=True,
            )
            results = list((root / "target/bench").glob("*/results.tsv"))
            self.assertEqual(len(results), 1, result.stderr)
            self.assertTrue(results[0].with_name("manifest.json").is_file())
            yield root, results[0]

    def rows(self, results: Path) -> list[dict[str, str]]:
        with results.open(encoding="utf-8") as handle:
            return list(csv.DictReader(handle, delimiter="\t"))

    def test_correct_files_from_a_crashed_or_timed_out_harness_are_not_a_pass(self) -> None:
        for status in (7, 124):
            with self.subTest(status=status), self.fake_run(status) as (_, results):
                row = self.rows(results)[0]
                self.assertEqual(row["pass"], "0")
                self.assertEqual(row["check_pass"], "1")
                self.assertEqual(row["exit"], str(status))

    def test_zero_exit_with_a_failed_or_unfinished_turn_is_not_a_pass(self) -> None:
        for terminal in ("failed", "started"):
            with self.subTest(terminal=terminal), self.fake_run(0, terminal) as (_, results):
                row = self.rows(results)[0]
                self.assertEqual(row["pass"], "0")
                self.assertEqual(row["check_pass"], "1")

    def test_a_source_edit_does_not_change_the_running_campaign_grader(self) -> None:
        with self.fake_run(0, mutate_grader=True) as (_, results):
            self.assertEqual(self.rows(results)[0]["pass"], "1")
            manifest = json.loads(results.with_name("manifest.json").read_text())
            for file in ("bench/lib/check.py", "bench/lib/test_receipts.py", "bench/lib/test_runner.py"):
                self.assertIn(file, manifest["files_sha256"])
            receipt = next(results.parent.glob("*.check.tests.result.json"))
            self.assertTrue(json.loads(receipt.read_text())["passed"])

    def test_zero_exit_without_coding_check_receipts_is_not_a_pass(self) -> None:
        with self.fake_run(0, missing_receipt=True) as (_, results):
            row = self.rows(results)[0]
            self.assertEqual(row["pass"], "0")
            self.assertEqual(row["check_pass"], "0")
            log = next(results.parent.glob("*.check"))
            self.assertIn("did not retain valid completion artifacts", log.read_text())

    def test_rescore_preserves_grading_and_original_rows(self) -> None:
        with self.fake_run() as (root, results):
            original = results.read_bytes()
            subprocess.run(["bash", str(root / "scripts/task-bench.sh"), "--rescore", results.parent.name],
                           capture_output=True, text=True, check=True)
            self.assertEqual(self.rows(results)[0]["check_pass"], "1")
            self.assertEqual(self.rows(results)[0]["pass"], "0")
            self.assertEqual(results.with_name("results.original.tsv").read_bytes(), original)
            self.assertTrue(results.with_name("rescore.json").is_file())

    def test_legacy_log_identity_survives_repeated_rescoring(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            results = root / "results.tsv"
            row = run_row()
            del row["kind"], row["run"]
            with results.open("w", encoding="utf-8", newline="") as handle:
                writer = csv.DictWriter(handle, fieldnames=list(row), delimiter="\t")
                writer.writeheader()
                writer.writerow(row)
            (root / "codex-bugfix.log").write_text(json.dumps({
                "type": "turn.completed", "usage": {"input_tokens": 100, "output_tokens": 1}
            }), encoding="utf-8")
            for _ in range(2):
                subprocess.run([sys.executable, str(ROOT / "bench/lib/rescore.py"), str(root)],
                               capture_output=True, text=True, check=True)
                rescored = self.rows(results)[0]
                self.assertEqual(rescored["pass"], "1")
                self.assertEqual(rescored["tokens"], "101")
                self.assertEqual(rescored["log_stem"], "codex-bugfix")


@unittest.skipUnless(hasattr(os, "fork"), "process-group test requires POSIX")
class TimeoutTests(unittest.TestCase):
    def test_timeout_kills_a_descendant_that_ignores_sigterm(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            worker = root / "worker.py"
            worker.write_text(
                "import os, pathlib, signal, sys\n"
                "reader, writer = os.pipe()\n"
                "pid = os.fork()\n"
                "if pid == 0:\n"
                "    signal.signal(signal.SIGTERM, signal.SIG_IGN)\n"
                "    os.write(writer, b'ready')\n"
                "    while True: signal.pause()\n"
                "os.read(reader, 5)\n"
                "pathlib.Path(sys.argv[1]).write_text(str(pid))\n"
                "signal.pause()\n", encoding="utf-8",
            )
            pid_file = root / "child.pid"
            result = subprocess.run([
                sys.executable, str(ROOT / "bench/lib/run.py"), "--timeout", "1",
                "--time-file", str(root / "wall"), "--cwd", str(root),
                "--stdout", str(root / "out"), "--stderr", str(root / "err"),
                "--", sys.executable, str(worker), str(pid_file),
            ], capture_output=True, text=True, timeout=15, check=False)
            self.assertEqual(result.returncode, 124, result.stderr)
            child = int(pid_file.read_text(encoding="utf-8"))
            try:
                state = subprocess.run(["ps", "-p", str(child), "-o", "stat="],
                                       capture_output=True, text=True, check=False).stdout.strip()
                self.assertTrue(not state or state.startswith("Z"), f"descendant {child} survived timeout: {state}")
            finally:
                with suppress(ProcessLookupError):
                    os.kill(child, signal.SIGKILL)


if __name__ == "__main__":
    unittest.main()
