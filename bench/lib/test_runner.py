"""Run a frozen unittest suite and report which checks actually completed.

This runs with submitted Python in one interpreter. The receipt detects missing
execution and skips; it is not a security boundary against arbitrary Python.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import unittest
from pathlib import Path


class RecordedResult(unittest.TextTestResult):
    def startTestRun(self) -> None:
        super().startTestRun()
        self.started: list[str] = []
        self.finished: list[str] = []
        self.succeeded: list[str] = []

    def startTest(self, test: unittest.TestCase) -> None:
        self.started.append(test.id())
        super().startTest(test)

    def stopTest(self, test: unittest.TestCase) -> None:
        self.finished.append(test.id())
        super().stopTest(test)

    def addSuccess(self, test: unittest.TestCase) -> None:
        self.succeeded.append(test.id())
        super().addSuccess(test)


class RecordedRunner(unittest.TextTestRunner):
    def _makeResult(self) -> RecordedResult:
        return RecordedResult(self.stream, self.descriptions, self.verbosity)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("receipt_fd", type=int)
    parser.add_argument("--root", type=Path)
    parser.add_argument("--test", action="append", default=[])
    args = parser.parse_args()
    receipt_fd = args.receipt_fd
    if args.root is not None:
        sys.path.insert(0, str(args.root.resolve()))
    suite = (
        unittest.defaultTestLoader.loadTestsFromNames(args.test)
        if args.test
        else unittest.defaultTestLoader.discover("_bench_hidden", top_level_dir=".")
    )
    result = RecordedRunner(verbosity=2).run(suite)
    if not isinstance(result, RecordedResult):
        raise TypeError(
            "Unittest did not use the configured completion-receipt result class."
        )
    receipt = {
        "tests_run": result.testsRun,
        "started": result.started,
        "finished": result.finished,
        "succeeded": result.succeeded,
        "failures": len(result.failures),
        "errors": len(result.errors),
        "skipped": len(result.skipped),
        "expected_failures": len(result.expectedFailures),
        "unexpected_successes": len(result.unexpectedSuccesses),
    }
    with os.fdopen(receipt_fd, "w", encoding="utf-8") as channel:
        json.dump(receipt, channel)
        channel.flush()
    return (
        0
        if result.wasSuccessful() and not result.skipped and not result.expectedFailures
        else 1
    )


if __name__ == "__main__":
    sys.exit(main())
