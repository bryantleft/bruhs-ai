"""Regression checks for ownership and failed-start cleanup, using fake server commands."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import database_fixture as fixture


class FixtureLifecycle(unittest.TestCase):
    def setup_paths(self, directory):
        binary = directory / "bin"
        binary.mkdir()
        for name in ("initdb", "pg_ctl", "createdb", "psql", "postgres"):
            (binary / name).touch()
        root = directory / "bruhs-ticket-pg-test"
        root.mkdir()
        return binary, root, directory / "receipt.json"

    def test_failed_start_with_live_pid_stops_before_removing(self):
        with tempfile.TemporaryDirectory() as temporary:
            binary, root, output = self.setup_paths(Path(temporary))
            calls = []

            def command(args, **_):
                calls.append(args)
                if args[0].endswith("initdb"):
                    (root / "data").mkdir()
                elif args[-1] == "start":
                    (root / "data/postmaster.pid").write_text("12345")
                    raise TimeoutError("Synthetic startup timeout after postmaster creation")
                elif args[-1] == "stop":
                    self.assertTrue(root.exists())
                return ""

            with patch.object(fixture.tempfile, "mkdtemp", return_value=str(root)), patch.object(fixture, "command", side_effect=command):
                with self.assertRaises(TimeoutError):
                    fixture.start(binary, output)
            self.assertEqual(calls[-1][-1], "stop")
            self.assertFalse(root.exists())

    def test_unconfirmed_stop_retains_root_and_private_recovery_receipt(self):
        with tempfile.TemporaryDirectory() as temporary:
            binary, root, output = self.setup_paths(Path(temporary))

            def command(args, **_):
                if args[0].endswith("initdb"):
                    (root / "data").mkdir()
                elif args[-1] == "start":
                    (root / "data/postmaster.pid").write_text("12345")
                    raise TimeoutError("Synthetic startup timeout")
                elif args[-1] == "stop":
                    raise TimeoutError("Synthetic stop timeout")
                return ""

            with patch.object(fixture.tempfile, "mkdtemp", return_value=str(root)), patch.object(fixture, "command", side_effect=command):
                with self.assertRaisesRegex(RuntimeError, "retained"):
                    fixture.start(binary, output)
            self.assertTrue((root / "data/postmaster.pid").exists())
            self.assertEqual(json.loads(output.read_text())["state"], "cleanup_required")
            self.assertEqual(output.stat().st_mode & 0o777, 0o600)

    def test_mismatched_marker_never_stops_or_removes_a_directory(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            binary, root, output = self.setup_paths(directory)
            (root / ".bruhs-database-fixture").write_text("another-owner")
            (root / "keep").write_text("retained")
            output.write_text(json.dumps({"engine": "native", "root": str(root), "name": root.name, "pg_bin": str(binary)}))
            with patch.object(fixture, "command") as command:
                with self.assertRaisesRegex(RuntimeError, "ownership marker"):
                    fixture.stop(output, remove=True)
                command.assert_not_called()
            self.assertEqual((root / "keep").read_text(), "retained")

    def test_existing_receipt_is_not_overwritten(self):
        with tempfile.TemporaryDirectory() as temporary:
            binary, _root, output = self.setup_paths(Path(temporary))
            output.write_text("retain existing file")
            with patch.object(fixture, "command") as command:
                with self.assertRaisesRegex(RuntimeError, "existing files are retained"):
                    fixture.start(binary, output)
                command.assert_not_called()
            self.assertEqual(output.read_text(), "retain existing file")


if __name__ == "__main__":
    unittest.main()
