#!/usr/bin/env bash
# The project's merge gate, run locally. GitHub Actions only runs when
# started by hand (the repo is private and Actions minutes cost money), so
# this script is the check that keeps `main` green.
#
#   scripts/gate.sh          # everything (as CI would run it)
#   scripts/gate.sh --quick  # fmt + clippy + tests only
#
# Each step runs on its own and its exit status is checked directly; output
# goes to target/gate/<step>.log. Exits non-zero if any step failed.
set -u

cd "$(dirname "$0")/.."

quick=0
if [ "${1:-}" = "--quick" ]; then
  quick=1
fi

logs=target/gate
mkdir -p "$logs"
export CARGO_TERM_COLOR=never
failed=()

step() {
  local name=$1
  shift
  printf '%-16s ' "$name"
  if "$@" >"$logs/$name.log" 2>&1; then
    echo ok
  else
    echo "FAILED (see $logs/$name.log)"
    failed+=("$name")
  fi
}

step fmt cargo fmt --all --check
step clippy cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
step clippy-wasm cargo clippy -p bruhs-web --target wasm32-unknown-unknown --locked -- -D warnings
step test cargo test --workspace --locked

# Harness benchmark fixtures: each hidden check fails on the starting repo
# and passes on its reference solution. No agent runs, so no tokens.
step bench-check sh -c 'bash -n scripts/task-bench.sh scripts/token-bench.sh && scripts/task-bench.sh --self-check'

if [ "$quick" -eq 0 ]; then
  step wasm-build cargo build -p bruhs-web --target wasm32-unknown-unknown --locked
  step desktop-release cargo check -p bruhs-desktop --release --locked
  step rustdoc env RUSTDOCFLAGS="-D warnings" \
    cargo doc --workspace --exclude bruhs-web --no-deps --locked
  if command -v cargo-deny >/dev/null 2>&1; then
    step deny cargo deny check
  else
    printf '%-16s skipped (install with `cargo install cargo-deny`)\n' deny
  fi
  if command -v trunk >/dev/null 2>&1; then
    step trunk-build sh -c 'cd bruhs-web && trunk build --release --dist ../target/gate/dist'
  else
    printf '%-16s skipped (trunk not installed)\n' trunk-build
  fi
fi

if [ "${#failed[@]}" -gt 0 ]; then
  echo "gate failed: ${failed[*]}"
  exit 1
fi
echo "gate passed"
