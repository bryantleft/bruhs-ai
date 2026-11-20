"""Decline every interactive request in an explicitly non-interactive cohort.

This local fixture adapter grants no authority and makes no coding decisions.
Native Claude print mode similarly returns denials instead of awaiting a person.
"""

from __future__ import annotations

import hashlib
import http.client
import json
import os
import subprocess
import time
from pathlib import Path


class PermissionDenier:
    def __init__(self, port: int, stream: Path, audit: Path) -> None:
        self.port = port
        self.stream = stream
        self.audit = audit
        self.offset = 0
        self.seen: set[str] = set()

    def poll(self) -> None:
        with self.stream.open("rb") as source:
            source.seek(self.offset)
            while True:
                line = source.readline()
                if not line.endswith(b"\n"):
                    return
                self.offset = source.tell()
                if b'"permission_request"' not in line:
                    continue
                if line.startswith(b"data: "):
                    line = line[6:]
                event = json.loads(line)
                if (
                    event.get("envelope") != "query"
                    or event.get("type") != "permission_request"
                ):
                    continue
                identity = event.get("id")
                if not isinstance(identity, str) or not identity:
                    raise ValueError("Permission request has no identity")
                if identity in self.seen:
                    continue
                connection = http.client.HTTPConnection(
                    "127.0.0.1", self.port, timeout=5
                )
                try:
                    connection.request(
                        "POST",
                        "/api/agent/approve",
                        json.dumps({"requestId": identity, "approved": False}),
                        {"Content-Type": "application/json"},
                    )
                    response = connection.getresponse()
                    status, body = response.status, response.read()
                    if status != 200 or json.loads(body).get("success") is not True:
                        raise RuntimeError(
                            f"Evaluation permission denial failed: HTTP {status}"
                        )
                finally:
                    connection.close()
                row = {
                    "request_id": identity,
                    "tool": event.get("tool"),
                    "approved": False,
                    "http_status": status,
                    "input_sha256": hashlib.sha256(
                        json.dumps(event.get("input"), sort_keys=True).encode()
                    ).hexdigest(),
                    "monotonic_seconds": time.monotonic(),
                    "policy": "decline every interactive request; no coding feedback",
                }
                with self.audit.open("a") as out:
                    out.write(json.dumps(row) + "\n")
                    out.flush()
                    os.fsync(out.fileno())
                self.seen.add(identity)


def communicate_declining(
    process: subprocess.Popen[bytes],
    input_data: bytes,
    timeout: float,
    denier: PermissionDenier,
) -> None:
    deadline = time.monotonic() + timeout
    pending_input: bytes | None = input_data
    while True:
        denier.poll()
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise subprocess.TimeoutExpired(process.args, timeout)
        try:
            process.communicate(input=pending_input, timeout=min(0.1, remaining))
            denier.poll()
            return
        except subprocess.TimeoutExpired:
            pending_input = None
