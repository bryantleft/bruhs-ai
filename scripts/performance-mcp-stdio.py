#!/usr/bin/env python3
"""Measure MCP process startup and stdio round trips with isolated synthetic data.

By default the bridge responder is synthetic. The ignored native GPUI fixture
can supply --native-connection to include the real foreground query handler.
Run optimized executables directly, without concurrent builds. Neither mode
measures paint, providers, or cold filesystem/page-cache startup.
"""

import argparse
import asyncio
import hashlib
import json
import math
import os
from pathlib import Path
import secrets
import struct
import tempfile
import time


ROOT = Path(__file__).resolve().parent.parent


def frame(method, params=None, request_id=1):
    value = {"jsonrpc": "2.0", "id": request_id, "method": method}
    if params is not None:
        value["params"] = params
    return json.dumps(value, separators=(",", ":")).encode() + b"\n"


def summary(samples):
    ordered = sorted(samples)
    return {
        f"p{percent}": ordered[math.ceil(len(ordered) * percent / 100) - 1]
        for percent in (50, 95, 99)
    }


class Client:
    def __init__(self, process):
        self.process = process

    @classmethod
    async def start(cls, binary, connection):
        env = dict(os.environ)
        env.pop("BRUHS_WORKSPACE_BRIDGE", None)
        if connection is not None:
            env["BRUHS_WORKSPACE_BRIDGE"] = str(connection)
        process = await asyncio.create_subprocess_exec(
            str(binary), stdin=asyncio.subprocess.PIPE,
            stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE,
            env=env, limit=1024 * 1024,
        )
        return cls(process)

    async def request(self, request):
        started = time.perf_counter_ns()
        self.process.stdin.write(request)
        await self.process.stdin.drain()
        reply = await asyncio.wait_for(self.process.stdout.readline(), 10)
        elapsed = (time.perf_counter_ns() - started) / 1_000_000
        assert reply.endswith(b"\n"), "missing complete response frame"
        decoded = json.loads(reply)
        assert decoded["jsonrpc"] == "2.0"
        assert decoded["id"] == json.loads(request)["id"]
        return decoded, elapsed, len(reply)

    async def close(self):
        self.process.stdin.close()
        try:
            await asyncio.wait_for(self.process.wait(), 10)
            stderr = await self.process.stderr.read()
            assert self.process.returncode == 0, stderr.decode(errors="replace")
        finally:
            if self.process.returncode is None:
                self.process.kill()
                await self.process.wait()


class BridgeFixture:
    """Use the real private-socket framing/authentication with a synthetic page."""

    async def __aenter__(self):
        self.directory = tempfile.TemporaryDirectory(prefix="bruhs-stdio-", dir="/tmp")
        directory = Path(self.directory.name)
        self.token = secrets.token_hex(32)
        socket = directory / "bridge.sock"
        self.connection = directory / "connection.json"
        fd = os.open(self.connection, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "w") as output:
            json.dump({"socket": str(socket), "token": self.token}, output)
        self.errors = []
        self.requests = 0
        self.server = await asyncio.start_unix_server(self.respond, path=socket)
        socket.chmod(0o600)
        return self

    async def __aexit__(self, *_):
        self.server.close()
        await self.server.wait_closed()
        self.directory.cleanup()
        assert not self.errors, self.errors

    async def respond(self, reader, writer):
        try:
            length = struct.unpack(">I", await reader.readexactly(4))[0]
            assert 0 < length <= 512 * 1024
            envelope = json.loads(await reader.readexactly(length))
            assert secrets.compare_digest(envelope["token"], self.token)
            request = envelope["request"]
            assert request["kind"] == "read_finance", request
            assert request["section"] == "transactions", request
            offset = request["offset"]
            query = request["query"]
            count = 0 if query == "missing" else 1 if query else 50
            start = 100_000 if query else offset
            records = [
                {"id": f"transaction-{i}", "merchant": f"Merchant {i}",
                 "category": "Category", "source": "manual"}
                for i in range(start, start + count)
            ]
            response = {"status": "ok", "value": {
                "records": records, "source": "workspace_cache",
                "next_offset": offset + count if not query and offset + count < 100_001 else None,
            }}
            encoded = json.dumps(response, separators=(",", ":")).encode()
            writer.write(struct.pack(">I", len(encoded)) + encoded)
            await writer.drain()
            self.requests += 1
        except Exception as error:
            self.errors.append(repr(error))
        finally:
            writer.close()
            await writer.wait_closed()


def tool(name, arguments=None):
    return frame("tools/call", {"name": name, "arguments": arguments or {}})


def check_result(reply, case):
    if case == "ping":
        assert reply["result"] == {}
    elif case == "tools-list":
        names = [entry["name"] for entry in reply["result"]["tools"]]
        assert len(names) == len(set(names)) and "bruhs_read_chat" in names
        return names
    elif case == "unknown-tool":
        assert reply["error"]["code"] == -32602
    elif case in ("chat-read-empty", "threads-list-empty", "thread-tasks-empty"):
        assert reply["result"].get("isError") is not True
        assert json.loads(reply["result"]["content"][0]["text"]) == []
    elif case == "agent-audit":
        assert reply["result"].get("isError") is not True
    elif case == "large-invalid-query":
        assert reply["result"]["isError"] is True
    else:
        assert reply["result"].get("isError") is not True, reply
        page = json.loads(reply["result"]["content"][0]["text"])
        count, first = {"finance-first": (50, 0), "finance-deep": (50, 99_950),
                        "finance-last": (1, 100_000), "finance-none": (0, None)}[case]
        assert len(page["records"]) == count
        if first is not None:
            assert [record["id"] for record in page["records"]] == [
                f"transaction-{index}" for index in range(first, first + count)
            ]
        assert page["next_offset"] == {"finance-first": 50, "finance-deep": 100_000,
                                        "finance-last": None, "finance-none": None}[case]
        assert all("amount_minor" not in record and "note" not in record
                   for record in page["records"])


async def measure(binary, mode, connection, startup_samples, samples):
    startup = []
    initialize = frame("initialize")
    # Fresh processes with warm OS caches. The first two launches are discarded.
    for iteration in range(startup_samples + 2):
        started = time.perf_counter_ns()
        client = await Client.start(binary, connection)
        try:
            reply, _, _ = await client.request(initialize)
            elapsed = (time.perf_counter_ns() - started) / 1_000_000
            assert reply["result"]["serverInfo"]["name"] == "bruhs-ai"
            if iteration >= 2:
                startup.append(elapsed)
        finally:
            await client.close()
    cases = {"ping": frame("ping"), "tools-list": frame("tools/list"),
             "unknown-tool": tool("bruhs_nonexistent_performance_fixture")}
    if mode == "standalone":
        cases.update({"agent-audit": tool("bruhs_agent_audit"),
                      "chat-read-empty": tool("bruhs_read_chat"),
                      "threads-list-empty": tool("bruhs_list_threads"),
                      "thread-tasks-empty": tool("bruhs_list_tasks", {"thread_id": "fixture-missing"})})
        cases["large-invalid-query"] = tool("bruhs_read_chat", {"query": "x" * 60_000})
    else:
        cases["large-invalid-query"] = tool("bruhs_read_finance", {
            "section": "transactions", "query": "x" * 60_000,
        })
        for label, query, offset in [("first", "", 0), ("deep", "", 99_950),
                                      ("last", "Merchant 100000", 0), ("none", "missing", 0)]:
            cases[f"finance-{label}"] = tool("bruhs_read_finance", {
                "section": "transactions", "query": query, "offset": offset,
            })
    results = {"fresh-process-initialize": {"samples_ms": startup,
                                             "summary_ms": summary(startup)}}
    client = await Client.start(binary, connection)
    try:
        await client.request(initialize)
        for case, request in cases.items():
            timings = []
            sizes = set()
            fingerprints = set()
            for iteration in range(samples + 10):
                reply, elapsed, size = await client.request(request)
                names = check_result(reply, case)
                if case == "tools-list":
                    assert ("bruhs_read_finance" in names) == (mode != "standalone")
                    assert ("bruhs_list_threads" in names) == (mode == "standalone")
                if iteration >= 10:
                    timings.append(elapsed)
                    sizes.add(size)
                    fingerprints.add(hashlib.sha256(json.dumps(reply, sort_keys=True,
                                                              separators=(",", ":")).encode()).hexdigest())
            results[case] = {"samples_ms": timings, "summary_ms": summary(timings),
                             "response_bytes": sorted(sizes),
                             "response_sha256": sorted(fingerprints)}
    finally:
        await client.close()
    return results


async def run(args):
    binary = args.binary.resolve(strict=True)
    result = {"binary": str(binary), "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
              "harness_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              "scope": "Fresh process warm OS caches; warm stdio complete-frame round trips. "
                       + ("Bridge includes the native GPUI fixture handler. " if args.native_connection
                          else "Bridge uses a synthetic Python responder, not the native handler. ")
                       + "Request encoding and response JSON validation are outside warm timers.",
              "started_unix_ns": time.time_ns(), "modes": {}}
    if args.native_connection:
        result["modes"]["bridge-native"] = await measure(binary, "bridge-native", args.native_connection,
                                                        args.startup_samples, args.samples)
    else:
        result["modes"]["standalone"] = await measure(binary, "standalone", None,
                                                     args.startup_samples, args.samples)
        async with BridgeFixture() as fixture:
            result["modes"]["bridge-fixture"] = await measure(binary, "bridge-fixture", fixture.connection,
                                                             args.startup_samples, args.samples)
            result["synthetic_bridge_requests"] = fixture.requests
    result["finished_unix_ns"] = time.time_ns()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({mode: {case: value["summary_ms"] for case, value in cases.items()}
                      for mode, cases in result["modes"].items()}, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--native-connection", type=Path,
                        help="Only the isolated native GPUI fixture should supply this connection")
    parser.add_argument("--startup-samples", type=int, default=25)
    parser.add_argument("--samples", type=int, default=200)
    options = parser.parse_args()
    if options.startup_samples < 1 or options.samples < 1:
        parser.error("sample counts must be positive")
    asyncio.run(run(options))
