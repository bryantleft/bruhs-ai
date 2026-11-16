"""Read the private evidence ledger through its compact public index."""

import hashlib
import json
from pathlib import Path
import re

PUBLIC_SUMMARY = "docs/performance-action-evidence.json"
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


def make_summary(root, archive, ledger, counts):
    """Create a small index; keep raw arrays and machine provenance private."""
    root = Path(root).resolve()
    archive = Path(archive).resolve()
    if not archive.is_relative_to(root / "target"):
        raise ValueError("Archive the full ledger under target/ before publishing an index.")
    raw = archive.read_bytes()
    _full_ledger(ledger)
    entries = []
    for entry in ledger["entries"]:
        entries.append({
            "evidence_id": entry["evidence_id"],
            "kind": entry["kind"],
            "id": entry["id"],
            "status": entry["status"],
            "report": entry["report"],
        })
    result = {
        "version": 1,
        "kind": SUMMARY_KIND,
        "purpose": "Compact publication index. Complete local provenance and raw arrays remain in the ignored archive.",
        "semantic_inventory_complete": False,
        "full_local_ledger": {
            "path": archive.relative_to(root).as_posix(),
            "sha256": hashlib.sha256(raw).hexdigest(),
            "bytes": len(raw),
            "git_tracked": False,
        },
        "public_reports": {
            "markdown": "docs/performance-action-log.md",
            "numerical": "docs/performance-action-log.json",
            "catalogue": "docs/performance-action-catalogue.json",
        },
        "counts": {
            "partial_evidence_records": len(entries),
            "normalized_timing_groups": counts["normalized_observations"],
            "request_count_cases": counts["request_count_observations"],
            "response_body_observations": counts["response_body_byte_observations"],
            "whole_action_acceptances": counts["whole_action_acceptances"],
            "matched_t3_acceptances": counts["verified_faster_t3_targets"],
        },
        "entry_index": entries,
        "limits": [
            "This index is not a complete action inventory or performance acceptance.",
            "Published numerical records retain their original order, boundary and scope; raw-input hashes name the unchanged private inputs.",
            "Pass the public index or a full local ledger to --ledger. Regeneration requires the referenced local archive.",
        ],
    }
    # Report references are already relative in retained data. Refuse an
    # unexpected absolute report instead of silently changing its identity.
    for entry in entries:
        report = entry["report"]
        if not isinstance(report, str) or report.startswith(("/", "\\")):
            raise ValueError("Public evidence report references must be repo-relative.")
    assert_publishable(json.dumps(result))
    return result
