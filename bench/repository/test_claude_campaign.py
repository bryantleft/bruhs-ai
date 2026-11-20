"""Campaign provenance fails closed on missing models and replaced binaries."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from bench.repository.claude_campaign import (
    ROOT,
    audit_binaries,
    require_actual_model,
    server_path,
)
from bench.workbench.run import digest


class CampaignProvenance(unittest.TestCase):
    def test_missing_or_different_actual_model_is_rejected(self) -> None:
        for value in (None, "", "another-model"):
            with self.subTest(value=value), self.assertRaises(RuntimeError):
                require_actual_model(value, "declared-model")
        require_actual_model("declared-model", "declared-model")

    def test_cargo_build_outputs_including_symlinks_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            link = Path(temporary) / "server-link"
            for profile in ("debug", "release"):
                with self.subTest(profile=profile):
                    target = ROOT / "target" / profile / "bruhs-server"
                    with self.assertRaises(ValueError):
                        server_path(target)
                    link.symlink_to(target)
                    with self.assertRaises(ValueError):
                        server_path(link)
                    link.unlink()

    def test_file_replacement_marks_server_and_provider_drift_separately(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            server = Path(temporary) / "server"
            provider = Path(temporary) / "provider"
            server.write_bytes(b"original-server")
            provider.write_bytes(b"original-provider")
            server_hash, provider_hash = digest(server), digest(provider)
            self.assertEqual(server_path(server), server.resolve())
            self.assertTrue(
                all(
                    audit_binaries(
                        server, provider, server_hash, provider_hash
                    ).values()
                )
            )
            replacement = Path(temporary) / "replacement"
            replacement.write_bytes(b"rebuilt-server")
            replacement.replace(server)
            self.assertEqual(
                audit_binaries(server, provider, server_hash, provider_hash),
                {"server_binary_unchanged": False, "provider_binary_unchanged": True},
            )
            provider.write_bytes(b"updated-provider")
            self.assertFalse(
                any(
                    audit_binaries(
                        server, provider, server_hash, provider_hash
                    ).values()
                )
            )


if __name__ == "__main__":
    unittest.main()
