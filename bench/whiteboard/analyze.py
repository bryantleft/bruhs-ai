"""Fail-closed Chromium completed-frame extraction for the frozen protocol."""
import argparse
import collections
import gzip
import json
import math
import random
import statistics
from pathlib import Path


def analyze(events):
    markers = {e["name"]: e for e in events if e["name"].startswith("board-")}
    phases = ("pan", "single", "group")
    expected = {f"board-{phase}-{edge}" for phase in phases for edge in ("start", "end")}
    if not expected.issubset(markers):
        raise ValueError("Missing workload marks; no JavaScript timing fallback is allowed")
    pids = {markers[name]["pid"] for name in expected}
    if len(pids) != 1:
        raise ValueError("Workload marks came from multiple renderer processes")
    pid = pids.pop()
    pending, presented, dropped = {}, {}, []
    for event in sorted(events, key=lambda event: event["ts"]):
        if event["pid"] != pid or event["name"] != "PipelineReporter":
            continue
        key = json.dumps(event.get("id2"), sort_keys=True)
        if event["ph"] == "b":
            if key in pending:
                raise ValueError("Ambiguous overlapping pipeline reporter identity")
            pending[key] = event
        elif event["ph"] == "e":
            begin = pending.pop(key, None)
            if begin is None:
                continue  # Started before tracing.
            report = begin.get("args", {}).get("frame_reporter", {})
            state = report.get("state")
            if state == "STATE_DROPPED":
                dropped.append((begin["ts"], event["ts"]))
            if state != "STATE_PRESENTED_ALL":
                continue
            identity = (report["frame_source"], report["frame_sequence"])
            if identity in presented:
                raise ValueError("A fully presented frame was reported more than once")
            presented[identity] = (begin["ts"], event["ts"])
    result = {}
    for phase in phases:
        low = markers[f"board-{phase}-start"]["ts"] + 200_000
        high = markers[f"board-{phase}-end"]["ts"] - 100_000
        frames = [(a, b) for a, b in presented.values() if low <= a and b <= high]
        ends = sorted(b for _, b in frames)
        if len(ends) < 2:
            raise ValueError(f"{phase} has fewer than two fully presented frames")
        unique = sorted(set(ends))
        if len(unique) < 2:
            raise ValueError(f"{phase} has fewer than two distinct presentation feedback timestamps")
        intervals = [(b-a)/1000 for a, b in zip(ends, ends[1:])]
        result[phase] = {
            "completed_frames": len(ends), "feedback_timestamps": len(unique),
            "mean_completed_interval_ms": statistics.mean(intervals),
            "mean_unique_feedback_interval_ms": (unique[-1]-unique[0])/(len(unique)-1)/1000 if len(unique)>1 else None,
            "median_completed_interval_ms": statistics.median(intervals),
            "mean_pipeline_latency_ms": statistics.mean((b-a)/1000 for a,b in frames),
            "dropped_reporters": sum(low <= a and b <= high for a,b in dropped),
        }
    return {"renderer_pid": pid, "workloads": result,
            "score_ms": statistics.mean(row["mean_completed_interval_ms"] for row in result.values())}


def paired_summary(directory):
    pairs = collections.defaultdict(dict)
    for path in directory.glob("pair-*/member-*.summary.json"):
        row = json.loads(path.read_text())
        pairs[path.parent.name][row["renderer"]] = row
    ratios = []
    for pair, rows in sorted(pairs.items()):
        if set(rows) != {"rust", "tldraw"}:
            raise ValueError(f"Incomplete pair {pair}")
        if any(not math.isfinite(row["score_ms"]) or row["score_ms"] <= 0 for row in rows.values()):
            raise ValueError(f"Nonpositive or nonfinite score in {pair}")
        ratios.append(rows["rust"]["score_ms"]/rows["tldraw"]["score_ms"])
    if len(ratios) != 12:
        raise ValueError("The primary protocol requires exactly 12 independent complete pairs")
    rng = random.Random(20261114)
    resamples = sorted(math.exp(statistics.mean(math.log(rng.choice(ratios)) for _ in ratios)) for _ in range(10_000))
    return {"pairs": len(ratios), "ratios": ratios,
            "geometric_mean_ratio": math.exp(statistics.mean(map(math.log, ratios))),
            "bootstrap_95_percent": [resamples[249], resamples[9749]],
            "accept": resamples[9749] < 1}


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("path", type=Path)
    parser.add_argument("--pairs", action="store_true")
    args = parser.parse_args()
    if args.pairs:
        output = paired_summary(args.path)
    else:
        opener = gzip.open if args.path.suffix == ".gz" else open
        with opener(args.path, "rt") as handle:
            output = analyze(json.load(handle)["traceEvents"])
    print(json.dumps(output, indent=2))
