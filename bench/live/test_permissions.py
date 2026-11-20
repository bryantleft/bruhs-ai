"""Non-interactive cohort permission decisions cannot grant authority or stall."""

from __future__ import annotations

import http.server
import json
import subprocess
import sys
import tempfile
import threading
import unittest
from pathlib import Path

from bench.live.permissions import PermissionDenier, communicate_declining


class DenialAdapter(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.stream = self.root / "stream.jsonl"
        self.stream.write_text("")
        self.calls: list[dict] = []
        self.status = 200
        self.signal = self.root / "answered"
        owner = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_POST(self) -> None:
                payload = json.loads(
                    self.rfile.read(int(self.headers["Content-Length"]))
                )
                owner.calls.append({"path": self.path, **payload})
                self.send_response(owner.status)
                self.end_headers()
                self.wfile.write(b'{"success":true}')
                owner.signal.touch()

            def log_message(self, format: str, *args: object) -> None:
                pass

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.denier = PermissionDenier(
            self.server.server_port, self.stream, self.root / "audit.jsonl"
        )

    def tearDown(self) -> None:
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()
        self.temporary.cleanup()

    def request(self, identity: str = "owned-request") -> str:
        return (
            json.dumps(
                {
                    "envelope": "query",
                    "type": "permission_request",
                    "id": identity,
                    "tool": "Bash",
                    "input": {"command": "private synthetic fixture"},
                }
            )
            + "\n"
        )

    def test_every_request_is_denied_once_without_coding_guidance(self) -> None:
        self.stream.write_text(self.request() + self.request() + self.request("second"))
        self.denier.poll()
        self.denier.poll()
        self.assertEqual(
            self.calls,
            [
                {"path": "/api/agent/approve", "requestId": identity, "approved": False}
                for identity in ["owned-request", "second"]
            ],
        )
        audit = [
            json.loads(line) for line in self.denier.audit.read_text().splitlines()
        ]
        self.assertEqual(len(audit), 2)
        self.assertNotIn("private synthetic fixture", self.denier.audit.read_text())

    def test_partial_frames_wait_for_newline_and_nonquery_events_do_not_act(
        self,
    ) -> None:
        frame = self.request()
        self.stream.write_text(
            '{"type":"permission_request","id":"not-query"}\n' + frame[:-1]
        )
        self.denier.poll()
        self.assertEqual(self.calls, [])
        with self.stream.open("a") as out:
            out.write("\n")
        self.denier.poll()
        self.assertEqual(len(self.calls), 1)

    def test_invalid_identity_or_failed_response_is_visible(self) -> None:
        self.stream.write_text(self.request(""))
        with self.assertRaises(ValueError):
            self.denier.poll()
        self.stream.write_text(self.request())
        self.denier.offset = 0
        self.status = 500
        with self.assertRaises(RuntimeError):
            self.denier.poll()
        self.assertFalse(self.denier.audit.exists())

    def test_real_waiting_child_continues_after_http_denial(self) -> None:
        program = "import pathlib,time,sys; print(sys.argv[1],flush=True); p=pathlib.Path(sys.argv[2]); \nwhile not p.exists(): time.sleep(.01)"
        with self.stream.open("wb") as out:
            child = subprocess.Popen(
                [
                    sys.executable,
                    "-c",
                    program,
                    self.request().strip(),
                    str(self.signal),
                ],
                stdin=subprocess.PIPE,
                stdout=out,
                stderr=subprocess.PIPE,
            )
            try:
                communicate_declining(child, b"", 3, self.denier)
                self.assertEqual(child.returncode, 0)
                self.assertEqual(self.calls[0]["approved"], False)
            finally:
                if child.poll() is None:
                    child.kill()
                child.communicate()

    def test_unrelated_silent_child_still_hits_the_original_deadline(self) -> None:
        with self.stream.open("wb") as out:
            child = subprocess.Popen(
                [sys.executable, "-c", "import time; time.sleep(5)"],
                stdin=subprocess.PIPE,
                stdout=out,
            )
            try:
                with self.assertRaises(subprocess.TimeoutExpired):
                    communicate_declining(child, b"", 0.15, self.denier)
            finally:
                child.kill()
                child.communicate()
        self.assertEqual(self.calls, [])


if __name__ == "__main__":
    unittest.main()
