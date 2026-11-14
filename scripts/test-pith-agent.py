#!/usr/bin/env python3
"""Live bruhs-ai -> provider CLI -> Pith -> streamed answer acceptance test.

Uses the signed-in local provider and real tokens. No mocked model or scanner.
Only disposable fixture metadata is requested; the fixture is removed afterwards.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import tempfile
import time
import urllib.parse
import urllib.request
import uuid


TOOLS = {"pith_status", "pith_scan", "pith_cleanup_candidates", "pith_inspect_path", "pith_estimate_cleanup"}


def fingerprint(root):
    return {str(path.relative_to(root)): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in root.rglob("*") if path.is_file()}


def tool_data(content):
    """Both SDKs relay MCP text blocks, sometimes wrapped in the tool result."""
    if isinstance(content, str):
        return tool_data(json.loads(content))
    if isinstance(content, list):
        return tool_data("\n".join(block["text"] for block in content if block.get("type") == "text"))
    if "structuredContent" in content:
        return content["structuredContent"]
    if "content" in content:
        return tool_data(content["content"])
    return content


def validate_tool_call(event, root, target, scan_id=None):
    name = event["tool"]
    if name == "ToolSearch":
        query = event["input"].get("query", "")
        selected = query.removeprefix("select:").split(",")
        allowed = {f"mcp__pith__{tool}" for tool in TOOLS}
        assert query.startswith("select:") and selected and all(tool in allowed for tool in selected), "Discovery may select only the five Pith tools"
        return None
    if not name.startswith("mcp__pith__") or name.removeprefix("mcp__pith__") not in TOOLS:
        raise AssertionError(f"Unexpected non-Pith tool: {name}")
    if name.endswith("pith_scan"):
        assert event["input"].get("path") == str(root), "The model must scan only the fixture"
    if name.endswith("pith_cleanup_candidates"):
        assert scan_id and event["input"].get("scan_id") == scan_id, "Candidate pages must use the exact scan_id returned by the fixture scan"
    if name.endswith("pith_inspect_path"):
        assert event["input"].get("path") in {str(target), "/"}, "Inspection must stay in the fixture, except the explicit scope-refusal probe"
    if name.endswith("pith_estimate_cleanup"):
        paths = event["input"].get("paths", [])
        assert len(paths) == 2 and set(paths) == {str(target), str(target / "build.bin")}, "The estimate must actually include both overlapping paths"
    return name


def run(args, root):
    target = root / "project" / "target"
    target.mkdir(parents=True)
    (root / "project" / "Cargo.toml").write_text('[package]\nname = "fixture"\nversion = "0.0.0"\n')
    (target / "build.bin").write_bytes(b"rebuildable fixture\n" * 65536)
    personal = root / "Documents" / "keep-this.bin"
    personal.parent.mkdir()
    personal.write_bytes(b"personal fixture, keep\n" * 131072)
    before = fingerprint(root)
    allocated = (target / "build.bin").stat().st_blocks * 512
    prompt = f"""Test the connected Pith integration end to end using only Pith MCP tools.
Do not use shell, Read, filesystem, browser, search or any non-Pith tool.
Use only the five read-only, pre-approved Pith tools. Do not enter planning mode or write a plan file.
If tool discovery is needed, use ToolSearch only with select: and the exact mcp__pith__ tool names.
Work in this disposable fixture only: {root}
First call pith_status and verify configuredRoot is exactly {Path.home().resolve()}.
If it is not, stop without further tools and ask for that home-folder scope before testing.
Then sequentially: call pith_scan on exactly that fixture path,
pith_cleanup_candidates with that scan's exact scan_id, pith_inspect_path on {target}, then
pith_estimate_cleanup on both {target} and its nested build.bin in ONE set.
The nested selection must count once. Recommend generated target output before personal files.
Finally call pith_inspect_path on / to verify that the configured home-folder scope refuses it;
report the refusal without using another tool to inspect it. Nothing should be changed.
Return only a JSON object with these fields, using the actual tool values:
scan_id (from the fixture scan), scan_root, recommended_path, risk, allocated_bytes, reclaimable_bytes,
personal_file_action (keep), outside_scope_refused (boolean), caveat (a sentence explaining
Trash, shared blocks or snapshots). Never invent byte figures. If any tool fails unexpectedly,
say what failed rather than substituting shell commands or pretending this passed."""
    body = {"thread_id": str(uuid.uuid4()), "prompt": prompt, "fresh_session": True,
            "permission_mode": "default", "thread_access": "off"}
    if args.model:
        body["model"] = args.model
    request = urllib.request.Request(args.server_url.rstrip("/") + "/api/agent",
                                     json.dumps(body).encode(), {"Content-Type": "application/json"})
    deadline = time.monotonic() + args.timeout
    events, calls, results, answer = [], {}, {}, None
    text_parts, completed_usage = [], False
    confirmed_home_scope = False
    scan_id = None
    args.out.parent.mkdir(parents=True, exist_ok=True)
    with args.out.with_suffix(".events.jsonl").open("w") as log, urllib.request.urlopen(request, timeout=args.timeout) as response:
        for raw in response:
            # SSE keepalives must not extend the overall live-test deadline.
            if time.monotonic() >= deadline:
                raise TimeoutError("The live Pith agent test exceeded its overall request budget")
            if not raw.startswith(b"data:"):
                continue
            event = json.loads(raw[5:])
            events.append(event)
            log.write(json.dumps(event) + "\n")
            log.flush()
            kind = event.get("type")
            if kind == "tool_call":
                name = validate_tool_call(event, root, target, scan_id)
                if name is None:
                    continue
                if not name.endswith("pith_status"):
                    assert confirmed_home_scope, "Confirm the home-folder Pith scope before any filesystem probe"
                calls[event["id"]] = event
                print("tool:", name, flush=True)
            elif kind == "tool_result":
                results[event["tool_use_id"]] = event
                call = calls.get(event["tool_use_id"])
                if call and call["tool"].endswith("pith_status"):
                    status = tool_data(event["content"])
                    assert not event.get("is_error") and status.get("configuredRoot") == str(Path.home().resolve()), "The live test requires exactly a home-folder Pith scope, not / or a narrower folder"
                    confirmed_home_scope = True
                if call and call["tool"].endswith("pith_scan"):
                    scanned = tool_data(event["content"])
                    assert not event.get("is_error"), "The fixture scan must succeed before requesting candidates"
                    scan_id = scanned.get("scan_id")
                    assert isinstance(scan_id, str) and re.fullmatch(r"scan_[0-9a-f]{32}", scan_id), "Pith must return an explicit unpredictable scan ID"
            elif kind in {"permission_request", "secret_request", "thread_request", "error"}:
                raise AssertionError(f"Unexpected {kind}: {event.get('message', event.get('tool', ''))}")
            elif kind == "text":
                text_parts.append(event["text"])
            elif kind == "usage":
                completed_usage = True
            elif kind == "result":
                assert not event.get("is_error"), event.get("result", "Provider failed")
                answer = event.get("result", "")
    # Codex sends final text and usage, then closes the successful stream. Claude sends an explicit result event.
    # The SDK holds Codex usage until process exit; any process failure is emitted as an error above.
    if answer is None and completed_usage:
        answer = "".join(text_parts)
    assert answer, "The actual provider must return a final answer"
    seen = {call["tool"].removeprefix("mcp__pith__") for call in calls.values()}
    assert seen == TOOLS, f"Missing tools: {TOOLS - seen}"
    successful = {}
    boundary = False
    for call_id, call in calls.items():
        assert call_id in results, f"No streamed result for {call['tool']}"
        result = results[call_id]
        name = call["tool"].removeprefix("mcp__pith__")
        data = tool_data(result["content"])
        if name == "pith_inspect_path" and call["input"].get("path") == "/":
            boundary = result.get("is_error") is True and "outside" in data.get("error", "")
        elif not result.get("is_error"):
            successful[name] = data
    assert set(successful) == TOOLS, "Every tool must complete successfully on the fixture"
    assert boundary, "Pith must actually refuse an out-of-scope request"
    assert successful["pith_scan"]["root"] == str(root)
    assert successful["pith_cleanup_candidates"]["scan_id"] == scan_id
    candidates = successful["pith_cleanup_candidates"]["candidates"]
    first = candidates[0]
    assert first["path"] == str(target) and first["risk"] == "rebuildable"
    assert first["allocatedBytes"] == allocated
    assert successful["pith_estimate_cleanup"]["files"] == 1, "Nested paths must not double count"
    start = answer.find("{")
    assert start >= 0, "The final answer must include the requested evidence JSON"
    summary, _ = json.JSONDecoder().raw_decode(answer[start:])
    assert summary["scan_id"] == scan_id
    assert summary["scan_root"] == str(root)
    assert summary["recommended_path"] == str(target)
    assert summary["risk"] == "rebuildable"
    assert summary["allocated_bytes"] == allocated
    assert summary["reclaimable_bytes"] == successful["pith_estimate_cleanup"]["bytes"]
    assert summary["personal_file_action"] == "keep"
    assert summary["outside_scope_refused"] is True
    assert summary["caveat"]
    assert fingerprint(root) == before, "Neither the model nor Pith may change fixture contents"
    receipt = {"passed": True, "model": args.model or "app default", "tools": sorted(seen),
               "explicit_scan_id_verified": True,
               "tool_calls": len(calls), "fixture_unchanged": True, "scope_boundary_refused": boundary,
               "answer": summary, "events": events}
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps({key: value for key, value in receipt.items() if key != "events"}, indent=2))
    print("Receipt:", args.out)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server-url", required=True)
    parser.add_argument("--model", help="Omit to use bruhs-ai's current default")
    parser.add_argument("--out", type=Path, default=Path(tempfile.gettempdir()) / "bruhs-pith-e2e.json")
    parser.add_argument("--timeout", type=float, default=180, help="Live provider test request budget in seconds")
    args = parser.parse_args()
    if args.timeout <= 0:
        parser.error("--timeout must be positive")
    if urllib.parse.urlparse(args.server_url).hostname not in {"127.0.0.1", "localhost", "::1"}:
        parser.error("This acceptance test requires a local bruhs-ai server")
    # Under HOME because bruhs-ai runs project-less turns there, and the installed Pith connection grants this home.
    with tempfile.TemporaryDirectory(prefix=".bruhs-pith-e2e-", dir=Path.home()) as directory:
        run(args, Path(directory).resolve())


if __name__ == "__main__":
    main()
