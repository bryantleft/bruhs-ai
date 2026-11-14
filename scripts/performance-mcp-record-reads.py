#!/usr/bin/env python3
"""Measure full-record MCP reads against the isolated native GPUI fixture."""

import argparse
import asyncio
import hashlib
import json
from pathlib import Path
import runpy
import time


ROOT = Path(__file__).resolve().parent.parent
TRANSPORT = ROOT / "scripts/performance-mcp-stdio.py"
helpers = runpy.run_path(str(TRANSPORT))
Client, frame, tool, summary = (helpers[name] for name in ("Client", "frame", "tool", "summary"))


def check(reply, first, count, next_offset):
    assert reply["result"].get("isError") is not True, reply
    page = json.loads(reply["result"]["content"][0]["text"])
    assert len(page["records"]) == count
    assert page["next_offset"] == next_offset
    assert [record["id"] for record in page["records"]] == [
        f"record-{index}" for index in range(first, first + count)
    ]


async def run(args):
    binary = args.binary.resolve(strict=True)
    result = {
        "mcp_binary": str(binary),
        "mcp_binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "harness_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "transport_harness_sha256": hashlib.sha256(TRANSPORT.read_bytes()).hexdigest(),
        "shape": args.shape,
        "records_per_domain": 10_001,
        "scope": "Parent request write/drain through complete MCP stdout frame, real private "
                 "Unix socket and native foreground query. Request encoding and parent JSON "
                 "validation excluded. Warm synthetic records; no physical input/paint, "
                 "connection enablement, cold hydration or provider work.",
        "started_unix_ns": time.time_ns(),
        "cases": {},
    }
    domains = ["tasks", "calendar", "capture", "knowledge", "inbox"]
    if args.shape == "unicode-knowledge":
        domains = ["knowledge"]
    client = await Client.start(binary, args.connection)
    try:
        initialized, _, _ = await client.request(frame("initialize"))
        assert initialized["result"]["serverInfo"]["name"] == "bruhs-ai"
        for domain in domains:
            cases = [
                ("first", "", 0, 0, 50, 50),
                ("deep", "", 9_950, 9_950, 50, 10_000),
                ("last", "lastquartz", 0, 10_000, 1, None),
                ("none", "no-quartz-match", 0, 0, 0, None),
            ]
            if args.shape == "unicode-knowledge":
                cases += [
                    ("ascii-i", "i", 0, 0, 50, 50),
                    ("ascii-k", "k", 0, 0, 50, 50),
                    ("final-sigma", "ς", 0, 0, 50, 50),
                    ("nonfinal-sigma", "Σ", 0, 0, 0, None),
                    ("unicode-absent", "ζ-unseen", 0, 0, 0, None),
                    ("ambiguous-ascii-absent", "missing-kelvin", 0, 0, 0, None),
                    ("mixed-latin-hit", "CAFÉ", 0, 0, 50, 50),
                    ("mixed-expansion-hit", "İ K", 0, 0, 50, 50),
                    ("mixed-context-hit", "ΟΣ Café", 0, 0, 50, 50),
                    ("unicode-only-absent", "ζ", 0, 0, 0, None),
                    ("ambiguous-fragment-absent", "ζ-missing-kelvin", 0, 0, 0, None),
                    ("present-fragment-absent", "ζcontext", 0, 0, 0, None),
                    ("present-ambiguous-fragment-absent", "ζlinks", 0, 0, 0, None),
                ]
            for label, query, offset, first, count, next_offset in cases:
                request = tool(f"bruhs_read_{domain}", {"query": query, "offset": offset})
                timings, fingerprints, sizes = [], set(), set()
                for iteration in range(35):
                    reply, elapsed, size = await client.request(request)
                    check(reply, first, count, next_offset)
                    if iteration >= 5:
                        timings.append(elapsed)
                        sizes.add(size)
                        fingerprints.add(hashlib.sha256(json.dumps(
                            reply, sort_keys=True, separators=(",", ":")
                        ).encode()).hexdigest())
                assert len(fingerprints) == 1
                result["cases"][f"{domain}/{label}"] = {
                    "samples_ms": timings, "summary_ms": summary(timings),
                    "response_sha256": sorted(fingerprints), "response_bytes": sorted(sizes),
                }
    finally:
        await client.close()
    result["finished_unix_ns"] = time.time_ns()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"shape": args.shape, "cases": {
        name: case["summary_ms"] for name, case in result["cases"].items()
    }}, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--connection", type=Path, required=True,
                        help="Only the isolated native GPUI fixture should supply this connection")
    parser.add_argument("--shape", choices=["ascii", "unicode-knowledge"], required=True)
    asyncio.run(run(parser.parse_args()))
