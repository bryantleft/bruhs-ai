# Mail and agents: combined validation checkpoint

The frozen Mail and Always-On implementation passes the full 14-step repository
gate on 2026-11-15 UTC. This is a local integration checkpoint; Inbox and the
wider product goal remain partial.

| Test group | Passed | Failed |
| --- | ---: | ---: |
| Rust unit, integration and documentation tests | 3,339 | 0 |
| Web asset checks | 5 | 0 |
| Site Node tests | 72 | 0 |
| Mobile Node tests | 33 | 0 |
| Total | 3,449 | 0 |

Rust also reports 54 ignored tests. The gate separately passes formatting,
strict native/all-feature and actual-WASM Clippy, site/mobile types, benchmark
fixture checks, WASM build, desktop release check, rustdoc, dependency policy and
the default production Trunk build. No live model eval is part of this gate.

## Source and bundle scope

All 1,473 frozen native inputs and the broader repository input snapshot stayed
fixed throughout validation. The gate compiles the two final copy changes:
the singular “1 byte” label and memory's “1 to N” warning. The only additional
source correction is rustfmt's public re-export ordering in `api.rs`.

The immutable default capture has 298 assets and 579 frontend inputs, with
API `http://127.0.0.1:3001` and default features. Copy verification checks the
complete file set, SHA256s and nanosecond mtimes. The earlier accepted capture
is unchanged.

The focused browser copy check passes 7 assertions with zero
failures at a 390×844 CSS viewport; it does not emulate a touch pointer. Three
unrelated minimal-fixture startup DTO warnings are retained, not called clean.
This check does not replace or expand the
[earlier browser and physical macOS evidence](evidence/mail-compose-ui-2026-11-14.json).
That record retains its original bundle/binary scope, including the warm-app
network limitation and synthetic-provider boundary.

Exact receipts, source/bundle hashes, focused cases and cleanup are in the
[compact finalization evidence](evidence/mail-finalization-2026-11-15.json).

## Remaining acceptance

Pre-queue authorization/no-dispatch recovery, Sent reconciliation, the complete
reply/accessibility/platform matrix, and live consent/provider/delivery checks
remain open. The Always-On UI and service installation still need on-screen
acceptance, and its outstanding Codex follow-up case has not been rerun.
The private whiteboard correction remains unadopted. This record makes no
whole-product, T3, push or deployment claim.
