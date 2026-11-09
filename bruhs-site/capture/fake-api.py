"""Static SPA + loopback fake API for landing-page screenshots.

Usage: python3 fake-api.py <dist> <port> <seed.json>
Never proxies anywhere; CSP connect-src 'self' keeps the page on loopback.
Every /api request is logged to /__log (GET) so unknown routes are visible.
"""
import json, sys, threading, time
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer

DIST, PORT, SEED = sys.argv[1], int(sys.argv[2]), sys.argv[3]
seed = json.load(open(SEED))
MEETINGS = {m["id"]: m for m in seed["meetings"]}
SEGMENTS = {}
for s in seed["segments"]:
    SEGMENTS.setdefault(s["meeting_id"], []).append(s)
PEOPLE = {p["id"]: p for p in seed["people"]}
LOG, LOCK = [], threading.Lock()

AUTH = {"authenticated": True, "user": "alex@bruhs.dev", "subscription_type": "max", "auth_method": "oauth"}
CAPTURE = {"running": True, "recording": False, "meeting_id": None, "started_at_ms": None}

class Handler(SimpleHTTPRequestHandler):
    def __init__(self, *a, **k):
        super().__init__(*a, directory=DIST, **k)
    def log_message(self, *a):
        pass
    def end_headers(self):
        self.send_header("Content-Security-Policy", "connect-src 'self'")
        self.send_header("Cache-Control", "no-store")
        super().end_headers()
    def _json(self, code, body):
        data = json.dumps(body).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)
    def _body(self):
        n = int(self.headers.get("Content-Length") or 0)
        return json.loads(self.rfile.read(n) or b"{}") if n else {}
    def _log(self, method, code):
        with LOCK:
            LOG.append({"method": method, "path": self.path, "code": code})
    def do_GET(self):
        if self.path.startswith("/__log"):
            with LOCK:
                return self._json(200, LOG)
        if self.path.startswith("/api/"):
            return self.api("GET")
        return super().do_GET()
    def do_POST(self):
        return self.api("POST")
    def do_PATCH(self):
        return self.api("PATCH")
    def do_DELETE(self):
        return self.api("DELETE")

    def ok(self, method, body, code=200):
        self._log(method, code)
        return self._json(code, body)

    def api(self, method):
        path = self.path.split("?")[0]
        parts = path.strip("/").split("/")
        if path.endswith("/stream") and method == "GET":
            self._log(method, 200)
            return self.sse()
        if path.startswith("/api/agent/auth/"):
            return self.ok(method, AUTH)
        if path == "/api/capture/status":
            return self.ok(method, CAPTURE)
        if path == "/api/people" and method == "GET":
            return self.ok(method, list(PEOPLE.values()))
        if path == "/api/people/versions":
            entries = sorted(({"id": p["id"], "updated_at": p["updated_at"]} for p in PEOPLE.values()), key=lambda e: e["id"])
            return self.ok(method, {"entries": entries, "next_after": None})
        if path == "/api/people/profiles" and method == "POST":
            ids = self._body().get("ids", [])
            return self.ok(method, [PEOPLE[i] for i in ids if i in PEOPLE])
        if len(parts) == 3 and parts[:2] == ["api", "people"] and method == "GET":
            p = PEOPLE.get(parts[2])
            return self.ok(method, p) if p else self.ok(method, {"error": "not found"}, 404)
        if path == "/api/meetings" and method == "GET":
            return self.ok(method, list(MEETINGS.values()))
        if len(parts) >= 3 and parts[:2] == ["api", "meetings"]:
            mid = parts[2]
            if len(parts) == 3 and method == "GET":
                m = MEETINGS.get(mid)
                return self.ok(method, m) if m else self.ok(method, {"error": "Meeting was not found."}, 404)
            if len(parts) == 4 and parts[3] == "segments" and method == "GET":
                return self.ok(method, SEGMENTS.get(mid, []))
            if len(parts) == 4 and parts[3] == "segment-removals" and method == "GET":
                return self.ok(method, [])
            if len(parts) == 4 and parts[3] == "generations" and method == "GET":
                return self.ok(method, None)
        if path == "/api/calendar/google/state":
            return self.ok(method, {"connection": "disconnected", "last_synced_at": None, "last_error": None, "calendars": []})
        if path in ("/api/terminal/sessions", "/api/preview/servers") and method == "GET":
            return self.ok(method, [])
        if path == "/api/tasks" and method == "GET":
            return self.ok(method, [])
        return self.ok(method, {"error": f"No fixture route for {method} {path}"}, 404)

    def sse(self):
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()
        try:
            if self.path.startswith("/api/meetings/stream"):
                self.wfile.write(b'data: {"kind":"stream_opened"}\n\n')
            self.wfile.flush()
            while True:
                time.sleep(10)
                self.wfile.write(b": heartbeat\n\n")
                self.wfile.flush()
        except (BrokenPipeError, ConnectionResetError):
            pass

ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
