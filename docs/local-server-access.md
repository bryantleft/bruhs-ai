# Local companion-server browser boundary

The companion server binds to loopback. Its API is powerful and must not become
accessible to an arbitrary website simply because that website runs in a local
browser. This boundary is not authentication against another local application.

## Policy

- Validate Host before routing. Accept `localhost`, `127.0.0.1`, `[::1]`, and
  valid `.localhost` names (a local proxy such as portless forwards those),
  with valid optional ports. Reject foreign/rebinding authorities, credentials
  and duplicate Host headers.
- Browser Origin must be one valid HTTP(S) origin on `localhost`, `127.0.0.1`
  or `[::1]`, or exactly one of the explicitly configured runtime origins:
  `BRUHS_API_URL` and `BRUHS_WEB_ORIGIN` (the web app's own origin, e.g.
  `https://bruhs.localhost` behind portless; `scripts/dev.sh` sets it).
  Other `*.localhost` origins are no longer trusted implicitly: any local dev
  site can claim one, and this API now drives the capture daemon. Reject
  opaque `null`, foreign, duplicate, malformed, path-bearing and
  credential-bearing origins. A configured origin grants workspace access;
  configure only a trusted frontend. It does not permit a foreign Host or
  expose the bind socket.
- Reject cross-site API requests without Origin based on Fetch Metadata. CORS
  preflights and actual requests use the same origin policy; simple requests do
  not bypass the handler guard. Native clients without browser headers continue
  to work on loopback. Terminal WebSocket upgrades use the same strict parser
  and still require Origin.
- The exact Google OAuth callback GET can receive a cross-site navigation
  without Origin. Its existing handler checks an expiring, one-use state before
  exchanging the authorization code. Other methods/API paths have no exception.
  Navigating to the public app shell from an external link remains possible.
- API responses are `no-store`; allowed responses deny framing and MIME sniffing.
  No database/provider/capture operation is performed by this middleware.

This still trusts other loopback websites and non-browser local processes. It
does not replace future pairing/authentication or protect a compromised approved
origin. Remote reverse-proxy deployment is not certified by these checks.

## Browser Content-Security-Policy

`bruhs-web/index.html` carries a CSP `<meta>`: scripts only from the page's
own origin (third-party libraries are vendored, the WASM loader is the
external `js/boot.js`) plus `'wasm-unsafe-eval'`; `object-src 'none'`;
`base-uri 'self'`; `img-src 'self' data: blob:`, so no image loads from
another host (assistant markdown renders a remote image as a link to it,
because loading it would send a prompt-injected URL to that host with no
click). The API origin is baked into the WASM at build time
(`BRUHS_API_URL`), so `connect-src` allows `'self'` and every loopback and
`*.localhost` origin over http/https/ws/wss — any port — which covers the
default `http://127.0.0.1:3001`, `scripts/dev.sh` and portless names such as
`https://api.bruhs.localhost`. An API on any other host needs that host added
to `connect-src`. IPv6 literals cannot be named in a CSP source list, so an
API at `http://[::1]:…` is not reachable from the page.

## Capture daemon (`bruhs-capture`, 127.0.0.1:3002)

The daemon can start microphone and screen capture and serves transcripts, so
it authenticates its callers instead of trusting the loopback interface:

- At every launch (after binding its port) it writes a fresh token, 32 random
  bytes hex-encoded, to `capture/daemon-token` under the app's local data
  directory (`~/Library/Application Support/dev.bruhs.bruhs-ai` on macOS) or to
  `BRUHS_CAPTURE_TOKEN_FILE`. The file is 0600, replaced atomically. The
  default directory is created or tightened to 0700; for
  `BRUHS_CAPTURE_TOKEN_FILE`, missing directories are created 0700 and an
  existing one (such as `$HOME` or `/tmp`) keeps its mode.
- Every route except `GET /health` requires `Authorization: Bearer <token>`
  (constant-time comparison); anything else gets 401. `/health` stays open
  because clients probe it before the token exists.
- Native clients read the file on every request, so a restarted daemon's new
  token is picked up: bruhs-server (`bruhs_core::capture_client`, including
  the event stream and recovery pump) and the desktop app (which calls the
  daemon directly). They honor `BRUHS_CAPTURE_TOKEN_FILE` too.
- Browsers never hold the token. The web app calls bruhs-server's
  `POST /api/capture/start`, `POST /api/capture/stop` and
  `GET`/`POST /api/capture/activity-monitor`; the server's local-access guard
  above applies, then the server forwards the request with the token and
  relays the daemon's status and body unchanged — except that a daemon that
  is not running or not reachable (connect error or timeout) is 503, and any
  other transport failure, an invalid reply, or a daemon 401 (the server's
  token is stale or missing) is 502, so the browser never sees a 401 for the
  server's own credential. `GET /api/capture/status` follows the same rule:
  a daemon that is not running or not reachable reads `"running": false`,
  while a daemon that refuses the server's token (or answers with anything
  but its status) is 502 with an explanatory error, never "not running".
- The daemon itself refuses every request that carries an `Origin` header or
  a cross-site/same-site `Sec-Fetch-Site`, and serves no CORS headers, so a
  web page cannot reach it even before the token check. Host is still limited
  to loopback names.
- Recordings, transcripts, OCR and centroid files are written 0600 in 0700
  directories.
