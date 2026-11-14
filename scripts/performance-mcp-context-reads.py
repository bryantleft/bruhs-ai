#!/usr/bin/env python3
"""Read synthetic context only through the private native GPUI fixture."""
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
Client, frame, tool, summary = (helpers[k] for k in ["Client", "frame", "tool", "summary"])


def check(reply, target, case):
    assert "error" not in reply
    result = reply["result"]
    assert len(result["content"]) == 1 and result["content"][0]["type"] == "text"
    if case.get("error"):
        assert result["isError"] is True and result["content"][0]["text"]
        return
    assert "isError" not in result
    page = json.loads(result["content"][0]["text"])
    assert len(page["records"]) == case["count"] and page["next_offset"] == case["next"]
    assert "unshared-secret" not in json.dumps(page)
    for index, row in enumerate(page["records"]):
        n = case.get("first", 0) + index
        kind = target["identity"]
        if kind == "chat/messages":
            assert row["content"].startswith(f"Completed {n}:")
            assert "files" not in row and "checkpoint" not in row
            if n == target["records"] - 1:
                assert row["thinking"] == "Stored synthetic reasoning"
                assert row["error"] == "Stored synthetic interruption" and row["tool_calls"][0]["id"] == "stored-tool"
        elif kind == "meeting/transcript":
            assert row["id"] == f"segment-{n}" and row["meeting_id"] == "context-meeting"
            assert row["partial"] is (n == target["records"] - 1)
        elif kind == "person/observations":
            assert row["text"].startswith(f"Observation {n}:")
        elif kind == "person/items":
            assert row["id"] == f"item-{n}" and row["text"].startswith(f"Relationship {n}:")
        elif kind == "meeting/summary":
            assert row["meeting"]["id"] == "context-meeting" and "audio_path" not in row["meeting"]
            assert row["summary"]["tldr"].startswith("Summary 0:")
        elif kind == "meeting-note/context":
            assert row["id"] == "context-note" and row["body"].startswith("Note 0:")
        else:
            assert row["id"] == "context-person" and row["observation_count"] == target["records"]
            assert row["relationship"]["item_count"] == target["records"] and "voice_embedding" not in row


async def run(args):
    binary = args.binary.resolve(strict=True)
    target, cases = json.loads(args.target), json.loads(args.cases)
    data = {"mcp_binary": str(binary), "mcp_binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
            "harness_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
            "transport_harness_sha256": hashlib.sha256(TRANSPORT.read_bytes()).hexdigest(),
            "target": target, "samples_per_case": args.samples, "warmups_per_case": args.warmups,
            "scope": "Parent request write/drain through complete MCP stdout frame, fixed MCP process, actual private Unix socket and native foreground handler for valid requests. Invalid arguments may be rejected by MCP before native dispatch. Parent JSON parsing/assertions, fixture construction/loading, cold startup, providers and physical input/paint excluded.",
            "started_unix_ns": time.time_ns(), "cases": {}}
    client = await Client.start(binary, args.connection)
    try:
        await client.request(frame("initialize"))
        for case in cases:
            arguments = dict(target["args"])
            if target["paged"]:
                arguments["offset"] = case["offset"]
            if case.get("missing"):
                for key in ["thread_id", "meeting_id", "note_id", "person_id"]:
                    if key in arguments:
                        arguments[key] = "context-missing"
            request = tool(target["tool"], arguments)
            samples, sizes, fingerprints = [], set(), set()
            for iteration in range(args.warmups + args.samples):
                reply, elapsed, size = await client.request(request)
                check(reply, target, case)
                if iteration >= args.warmups:
                    samples.append(elapsed)
                    sizes.add(size)
                    fingerprints.add(hashlib.sha256(json.dumps(reply, sort_keys=True, separators=(",", ":")).encode()).hexdigest())
            assert len(fingerprints) == 1
            data["cases"][case["name"]] = {"descriptor": case, "samples_ms": samples, "summary_ms": summary(samples),
                                           "response_bytes": sorted(sizes), "response_sha256": sorted(fingerprints)}
    finally:
        await client.close()
    data["finished_unix_ns"] = time.time_ns()
    args.output.write_text(json.dumps(data, indent=2) + "\n")
    print(json.dumps({"target": target["identity"], "cases": {k: v["summary_ms"] for k, v in data["cases"].items()}}, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--connection", type=Path, required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--cases", required=True)
    parser.add_argument("--warmups", type=int, default=5)
    parser.add_argument("--samples", type=int, default=30)
    args = parser.parse_args()
    assert args.samples > 0 and args.warmups >= 0
    asyncio.run(run(args))
