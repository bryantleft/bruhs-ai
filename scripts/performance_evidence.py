"""Read the private evidence ledger through its compact public index."""

import hashlib
import json
from pathlib import Path
import re

PUBLIC_SUMMARY = "target/performance-action-evidence.json"
SUMMARY_KIND = "performance_evidence_summary"


def assert_publishable(text):
    """Reject accidental machine home paths before writing public reports."""
    if re.search(r"/(?:Users|home)/|[A-Za-z]:[\\]+Users[\\]+", text):
        raise ValueError("Public performance output contains an absolute home path.")


def _full_ledger(value):
    if (
        not isinstance(value, dict)
        or value.get("version") != 1
        or not isinstance(value.get("entries"), list)
        or not isinstance(value.get("landings"), list)
    ):
        raise ValueError("Expected the full local performance evidence ledger.")
    return value


def load_ledger(root, requested=PUBLIC_SUMMARY, *, required=True):
    """Return (full path, full data); a public summary alone is insufficient."""
    root = Path(root).resolve()
    path = root / requested
    if not path.is_file():
        if not required:
            return None
        raise FileNotFoundError("Full local performance evidence is required; supply --ledger PATH.")
    value = json.loads(path.read_bytes())
    if not isinstance(value, dict):
        raise ValueError("Expected a performance evidence object.")
    if value.get("kind") != SUMMARY_KIND:
        return path, _full_ledger(value)
    reference = value.get("full_local_ledger", {})
    relative = Path(reference.get("path", ""))
    if relative.is_absolute():
        raise ValueError("The private evidence archive reference must be repo-relative.")
    archive = root / relative
    if archive.is_symlink() or not archive.resolve().is_relative_to(root / "target"):
        raise ValueError("The private evidence archive must remain under target/.")
    if not archive.is_file():
        if not required:
            return None
        raise FileNotFoundError("The full ledger is retained locally under target/; the public index cannot recreate timing data.")
    raw = archive.read_bytes()
    if len(raw) != reference.get("bytes") or hashlib.sha256(raw).hexdigest() != reference.get("sha256"):
        raise ValueError("The private evidence archive does not match its recorded size and digest.")
    return archive, _full_ledger(json.loads(raw))
