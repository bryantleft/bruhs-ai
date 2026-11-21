"""The live read fixture must reject forge mutations before native gh starts."""

from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from bench.delivery.read_github import readonly_gh  # noqa: E402


@unittest.skipUnless(os.name == "posix", "owned CLI fixture uses POSIX executables")
class ReadOnlyDeliveryTests(unittest.TestCase):
    def invoke(self, args: list[str]) -> tuple[int, bool, dict]:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = root / "native-gh"
            binary.write_text("#!/bin/sh\nprintf ran > \"$HOME/native-started\"\n", encoding="utf-8")
            binary.chmod(0o700)
            wrapper, calls = root / "gh", root / "calls.jsonl"
            readonly_gh(binary, wrapper, calls, root)
            result = subprocess.run([str(wrapper), *args], capture_output=True, timeout=60,
                                    env={"HOME": str(root), "PATH": "/usr/bin:/bin",
                                         "GH_REPO": "github.com/fixture/repo", "GH_HOST": "github.com"})
            return result.returncode, (root / "native-started").exists(), json.loads(calls.read_text())

    def test_read_records_scope_and_starts_owned_cli(self) -> None:
        status, started, call = self.invoke(["api", "repos/{owner}/{repo}/pulls/10/comments?per_page=100", "--paginate", "--slurp"])
        self.assertEqual(status, 0)
        self.assertTrue(started)
        self.assertEqual(call["repository"], "github.com/fixture/repo")
        self.assertTrue(call["read_only_allowed"])

    def test_write_forms_are_rejected_before_native_cli_starts(self) -> None:
        endpoint = "repos/{owner}/{repo}/pulls/10/comments"
        for args in (["pr", "merge", "10"], ["pr", "comment", "10"],
                     ["api", endpoint, "--method", "POST"], ["api", endpoint, "--method=PUT"],
                     ["api", endpoint, "-XPOST"], ["api", endpoint, "-fbody=fixture"],
                     ["api", endpoint, "-Fbody=fixture"], ["api", endpoint, "--input", "-"],
                     ["api", "graphql", "-fquery=mutation{}"]):
            with self.subTest(args=args):
                status, started, call = self.invoke(args)
                self.assertNotEqual(status, 0)
                self.assertFalse(started)
                self.assertFalse(call["read_only_allowed"])


if __name__ == "__main__":
    unittest.main()
