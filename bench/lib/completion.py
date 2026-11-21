"""Check terminal provider events separately from a successful CLI or HTTP exit."""

from __future__ import annotations

from pathlib import Path
import sys

from metrics import read_json, read_jsonl, read_text, sibling
from review import holds_up


def completion(fmt: str, path: str) -> str:
    if not Path(path).is_file():
        return "unknown"
    if fmt.startswith("bruhs-review-"):
        provider = "bruhs-" + fmt.removeprefix("bruhs-review-")
        first = completion(provider, path)
        if first != "completed":
            return first
        http = read_text(sibling(path, ".review.http"))
        if http != "200":
            return "failed" if http else "unknown"
        review = read_json(sibling(path, ".review.json"))
        if not isinstance(review, dict) or review.get("error") or not review.get("review"):
            return "failed"
        if Path(sibling(path, ".t2.log")).exists():
            return completion(provider, sibling(path, ".t2.log"))
        return "completed" if holds_up(review["review"]) else "unknown"
    rows = read_jsonl(path)
    if fmt in ("claude", "bruhs-claude", "bruhs-codex"):
        if fmt.startswith("bruhs-"):
            rows = [row for row in rows if row.get("envelope") == "query"]
        terminal = [row for row in rows if row.get("type") in ("result", "error")
                    or (fmt == "bruhs-codex" and row.get("type") == "usage")]
        if not terminal:
            return "unknown"
        last = terminal[-1]
        if last.get("type") == "error" or last.get("is_error") is True:
            return "failed"
        if last.get("type") == "usage" or last.get("is_error") is False or last.get("subtype") == "success":
            return "completed"
        return "unknown"
    if fmt == "codex":
        terminal = [row for row in rows if row.get("type") in ("turn.completed", "turn.failed", "error")]
        if not terminal:
            return "unknown"
        return "completed" if terminal[-1]["type"] == "turn.completed" else "failed"
    if fmt.startswith("t3-"):
        last = rows[-1] if rows else {}
        if last.get("status") == "completed" and not last.get("error"):
            return "completed"
        return "failed" if last.get("error") or last.get("status") in ("failed", "cancelled", "interrupted") else "unknown"
    # These adapters expose only process completion to this benchmark.
    if fmt in ("aider", "opencode"):
        return "process-only"
    raise ValueError(f"unsupported benchmark format {fmt!r}")


if __name__ == "__main__":
    print(completion(*sys.argv[1:]))
