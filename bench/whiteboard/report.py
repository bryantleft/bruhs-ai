"""Aggregate completed cohorts without modifying their retained evidence."""
import argparse
import hashlib
import json
import statistics
from datetime import datetime, timezone
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument("root", type=Path)
parser.add_argument("date")
parser.add_argument("output", type=Path)
args = parser.parse_args()
result = {
    "schema": 1,
    "created_at": datetime.now(timezone.utc).isoformat(),
    "raw_evidence": str(args.root),
    "scope": "Instrumented completed compositor-frame intervals, not physical scanout or input latency.",
    "aggregation": "Arithmetic means of every member's workload mean; primary paired decision is copied unchanged.",
    "cohorts": {},
    "native": {},
}
for name, pairs in [("primary", 12), ("normal", 3), ("scale", 2), ("replay", 1)]:
    directory = args.root / f"{args.date}-{name}"
    manifest = json.loads((directory / "manifest.json").read_text())
    completion = json.loads((directory / "completion.json").read_text())
    if manifest["pairs"] != pairs or not completion["artifacts_unchanged"]:
        raise ValueError(f"Incomplete or altered cohort: {name}")
    cohort = {"pairs": pairs, "count": manifest["count"], "mode": manifest["mode"], "renderers": {}}
    for renderer in ["rust", "tldraw"]:
        metadata = [json.loads(p.read_text()) for p in sorted(directory.glob(f"pair-*/member-{renderer}.json"))]
        if len(metadata) != pairs or any(row.get("failure") or row["errors"] or len(row["validation"]) != 3 for row in metadata):
            raise ValueError(f"Invalid members: {name}/{renderer}")
        if name == "replay":
            continue
        rows = [json.loads(p.read_text()) for p in sorted(directory.glob(f"pair-*/member-{renderer}.summary.json"))]
        if len(rows) != pairs:
            raise ValueError(f"Missing summaries: {name}/{renderer}")
        cohort["renderers"][renderer] = {
            phase: {
                field: statistics.mean(row["workloads"][phase][field] for row in rows)
                for field in ["mean_completed_interval_ms", "mean_unique_feedback_interval_ms", "mean_pipeline_latency_ms", "completed_frames", "feedback_timestamps", "dropped_reporters"]
            }
            for phase in ["pan", "single", "group"]
        }
    result["cohorts"][name] = cohort
    if name == "primary":
        result["decision"] = json.loads((directory / "decision.json").read_text())
        result["hardware"] = manifest["hardware"]
        result["browser"] = metadata[0]["version"]
        result["gpu"] = metadata[0]["gpu"]
for count in [1000, 10000]:
    row = json.loads((args.root / f"native-{count}.json").read_text())
    if row["shapes"] != count:
        raise ValueError("Native probe omitted fixture records")
    result["native"][str(count)] = {
        "scope": row["timing_scope"], "viewport": row["viewport"], "dpr": row["dpr"],
        "mean_callback_interval_ms": {
            phase: statistics.mean(sample["callback_interval_ms"] for sample in row["samples"] if sample["phase"] == index)
            for index, phase in enumerate(["pan", "single", "group"])
        },
    }
result["artifacts"] = [
    {"path": str(path), "bytes": path.stat().st_size, "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
    for path in sorted(args.root.rglob("*")) if path.is_file()
]
args.output.parent.mkdir(parents=True, exist_ok=True)
with args.output.open("x") as output:
    json.dump(result, output, indent=2)
    output.write("\n")
print(json.dumps(result["decision"], indent=2))
