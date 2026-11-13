"""Release artifact contract, independent of the HTTP server implementation."""

import gzip
import importlib.util
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import unittest


HOOK = Path(__file__).resolve().parents[1] / "precompress.py"
SPEC = importlib.util.spec_from_file_location("precompress", HOOK)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class PrecompressTests(unittest.TestCase):
    def test_round_trip_preserves_originals_mime_extensions_and_metadata(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            content = b"release asset with repetitive source data\n" * 1024
            names = ["index.html", "boot.js", "module.mjs", "style.css",
                     "data.json", "icon.svg", "app.wasm"]
            for name in names:
                source = directory / name
                source.write_bytes(content)
                source.chmod(0o640)
                os.utime(source, ns=(1_700_000_000_000_000_000,
                                     1_700_000_001_000_000_000))
            (directory / "image.png").write_bytes(content)
            (directory / "font.woff2").write_bytes(content)
            result = MODULE.precompress(directory)
            self.assertEqual(result["files"], len(names))
            for name in names:
                source = directory / name
                sidecar = directory / (name + ".gz")
                self.assertEqual(source.read_bytes(), content)
                self.assertEqual(gzip.decompress(sidecar.read_bytes()), content)
                self.assertLess(sidecar.stat().st_size, source.stat().st_size)
                self.assertEqual(sidecar.stat().st_mtime_ns, source.stat().st_mtime_ns)
                self.assertEqual(stat.S_IMODE(sidecar.stat().st_mode), 0o640)
            self.assertFalse((directory / "image.png.gz").exists())
            self.assertFalse((directory / "font.woff2.gz").exists())

    def test_output_is_independent_of_filename_and_source_time(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            for name, timestamp in [("one.js", 1_600_000_000),
                                    ("different.js", 1_700_000_000)]:
                source = directory / name
                source.write_bytes(b"deterministic release payload" * 1024)
                os.utime(source, (timestamp, timestamp))
            MODULE.precompress(directory)
            first = (directory / "one.js.gz").read_bytes()
            self.assertEqual(first, (directory / "different.js.gz").read_bytes())
            MODULE.precompress(directory)
            self.assertEqual(first, (directory / "one.js.gz").read_bytes())

    def test_rerun_replaces_sidecars_and_removes_nonbeneficial_versions(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            source = directory / "app.wasm"
            sidecar = directory / "app.wasm.gz"
            source.write_bytes(b"old payload" * 1024)
            MODULE.precompress(directory)
            source.write_bytes(b"new payload" * 2048)
            MODULE.precompress(directory)
            self.assertEqual(gzip.decompress(sidecar.read_bytes()), source.read_bytes())
            source.write_bytes(b"")
            MODULE.precompress(directory)
            self.assertFalse(sidecar.exists())
            self.assertEqual(list(directory.iterdir()), [source])

    def test_release_hook_handles_spaces_and_debug_skips_compression(self):
        with tempfile.TemporaryDirectory(prefix="web assets ") as temporary:
            directory = Path(temporary)
            source = directory / "index.html"
            source.write_bytes(b"<p>release</p>" * 2048)
            environment = {**os.environ, "TRUNK_PROFILE": "debug",
                           "TRUNK_STAGING_DIR": str(directory)}
            subprocess.run([sys.executable, str(HOOK)], env=environment, check=True,
                           capture_output=True, text=True)
            self.assertFalse((directory / "index.html.gz").exists())
            environment["TRUNK_PROFILE"] = "release"
            subprocess.run([sys.executable, str(HOOK)], env=environment, check=True,
                           capture_output=True, text=True)
            self.assertEqual(gzip.decompress((directory / "index.html.gz").read_bytes()),
                             source.read_bytes())

    def test_missing_release_directory_fails_the_build(self):
        with tempfile.TemporaryDirectory() as temporary:
            environment = {**os.environ, "TRUNK_PROFILE": "release",
                           "TRUNK_STAGING_DIR": str(Path(temporary) / "missing")}
            result = subprocess.run([sys.executable, str(HOOK)], env=environment,
                                    capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Trunk staging directory is missing", result.stderr)


if __name__ == "__main__":
    unittest.main()
