#!/usr/bin/env bash
#
# Launch the full bruhs dev stack — capture daemon, API server, web SPA —
# in one terminal with unified logs and clean teardown on Ctrl-C.
#
# Portless by default: the web app and API are reached by stable name
# (https://bruhs.localhost, api.bruhs.localhost) via a shared proxy, so bruhs
# never fights other projects for ports. The capture daemon stays on loopback:
# only native clients (the server, the desktop app) talk to it, with the bearer
# token it writes at launch; the browser goes through the server's
# /api/capture/* proxy. The proxy
# binds :443 for clean port-free HTTPS (prompts once for sudo); without sudo
# it falls back to :1355 and the URLs carry that suffix — still collision-free.
#
# Bind ports honor BRUHS_{CAPTURE,SERVER,WEB}_PORT (the named routes are
# re-pointed to match). Set PORTLESS=0 to bypass the proxy and use plain
# 127.0.0.1 ports instead.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

SERVER_PORT="${BRUHS_SERVER_PORT:-3001}"
CAPTURE_PORT="${BRUHS_CAPTURE_PORT:-3002}"
WEB_PORT="${BRUHS_WEB_PORT:-8080}"

if [ "${PORTLESS:-1}" = "0" ]; then
  API_URL="http://127.0.0.1:$SERVER_PORT"
  WEB_OPEN="http://127.0.0.1:$WEB_PORT"
  # Loopback origins are always allowed; no extra origin to configure.
  WEB_ORIGIN=""
else
  # Point the named routes at whatever ports we're about to bind, then bring
  # up the shared proxy and ask it for the canonical URLs (scheme + port track
  # the running proxy: https://…  on :443, or http://…:1355 without sudo).
  portless alias bruhs "$WEB_PORT" --force >/dev/null
  portless alias api.bruhs "$SERVER_PORT" --force >/dev/null
  echo "→ starting portless proxy (may prompt once for sudo to bind :443)…"
  # An already-running proxy is not an error: portless prints "Proxy is already
  # running" and exits 0. A non-zero exit is a real failure (bad config, port
  # taken, sudo refused with an explicit port), so stop instead of carrying on
  # with URLs that nothing is serving.
  if ! portless proxy start; then
    echo "✗ portless proxy failed to start; fix the error above or rerun with PORTLESS=0" >&2
    exit 1
  fi
  API_URL="$(portless get api.bruhs)"
  WEB_OPEN="$(portless get bruhs)"
  # The server trusts a `*.localhost` browser origin only when configured.
  WEB_ORIGIN="${WEB_OPEN%/}"
fi

# The server→daemon hop is internal (Rust-to-Rust), so it stays on loopback —
# no reason to bounce it through the proxy.
SERVER_TO_DAEMON="http://127.0.0.1:$CAPTURE_PORT"

pids=()
names=()
cleanup() {
  # Idempotent: the INT/TERM handlers call `exit`, which fires the EXIT trap.
  trap - INT TERM EXIT
  echo ""
  echo "→ shutting down bruhs dev stack…"
  # `${arr[@]+"${arr[@]}"}` keeps an empty array safe under `set -u` on bash 3.2.
  for pid in ${pids[@]+"${pids[@]}"}; do kill "$pid" 2>/dev/null || true; done
  wait 2>/dev/null || true
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

start() { # start <name> <command…>: run in the background and remember the pid
  local name="$1"
  shift
  "$@" &
  pids+=("$!")
  names+=("$name")
}

echo "→ capture daemon  :$CAPTURE_PORT"
BRUHS_CAPTURE_PORT="$CAPTURE_PORT" start capture cargo run -p bruhs-capture

echo "→ api server      :$SERVER_PORT  (daemon at :$CAPTURE_PORT)"
BRUHS_SERVER_PORT="$SERVER_PORT" \
  BRUHS_CAPTURE_URL="$SERVER_TO_DAEMON" \
  BRUHS_WEB_ORIGIN="$WEB_ORIGIN" \
  start server cargo run -p bruhs-server

echo "→ web SPA         :$WEB_PORT  →  $WEB_OPEN"
# `exec` makes the subshell become trunk, so the pid we kill on teardown is
# trunk itself rather than a wrapper that would leave it running.
(
  cd bruhs-web &&
    BRUHS_API_URL="$API_URL" \
    exec trunk serve --port "$WEB_PORT"
) &
pids+=("$!")
names+=("web")

echo ""
echo "→ open $WEB_OPEN   (Ctrl-C stops all three)"

# Supervise the stack: all three are long-running, so any of them exiting
# (even with status 0) means the stack is broken. Report which one died, tear
# the rest down via the EXIT trap, and exit non-zero.
status=0
dead=""
# `wait -n` needs bash 4.3+; macOS ships bash 3.2, so poll `kill -0` there.
if ((BASH_VERSINFO[0] > 4 || (BASH_VERSINFO[0] == 4 && BASH_VERSINFO[1] >= 3))); then
  wait -n || status=$?
  for i in "${!pids[@]}"; do
    if ! kill -0 "${pids[$i]}" 2>/dev/null; then
      dead="${names[$i]}"
      break
    fi
  done
else
  while [ -z "$dead" ]; do
    for i in "${!pids[@]}"; do
      if ! kill -0 "${pids[$i]}" 2>/dev/null; then
        dead="${names[$i]}"
        wait "${pids[$i]}" || status=$?
        break
      fi
    done
    [ -n "$dead" ] || sleep 1
  done
fi

echo "✗ ${dead:-a service} exited (status $status); stopping the rest of the stack" >&2
if [ "$status" -eq 0 ]; then status=1; fi
exit "$status"
