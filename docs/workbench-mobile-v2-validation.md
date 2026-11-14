# Workbench and native companion validation

The core baseline below was validated locally on 2026-11-13, with mobile build
follow-ups on 2026-11-14. The final combined workspace gate passes: 3,010 tests,
zero failures and 54 ignored, with strict native/WASM lint, release checks,
rustdoc, dependency policy and the default production Trunk build. Managed
compute, relays, cloud speech and cloud mobile builds are
excluded. Desktop-owned mobile implementation remains under BNLE-68; this pass
consumes its API without editing those files.

| Check | Result |
| --- | --- |
| Full core unit suite | 667 passed, 0 failed, 10 ignored |
| Full models unit suite | 475 passed, 0 failed, 1 ignored |
| HTTP authority/origin/delivery checks | 9 passed, 0 failed |
| Real installed OpenCode ACP | v1 negotiation, native Plan selection and session reload pass without a prompt/provider turn |
| Five-crate strict Clippy | Core, models, server, web and desktop, all targets, warnings denied: pass |
| WASM target | `cargo check -p bruhs-web --target wasm32-unknown-unknown`: pass |
| Mobile local merge gate | 33 protocol/storage/race/signing-log tests and strict TypeScript: pass |
| Native iOS | Local Debug build and actual simulator pairing, Keychain persistence, exact approval retry, prompts, diffs, offline/reconnect and revocation: pass |
| Mobile exports | Web production and native iOS Hermes exports: pass |
| Android | Local Debug APK build and 15 synthetic native checks: pass, using the already installed SDK/Java discovered outside PATH. Final current-source 100%/200% keyboard, approval/retry, draft/diff and recovery checks also pass; see the native guide. |
| iOS signing | Current-layout Release archive rebuilt at 2026-11-14 10:15:23 UTC; deep/strict signature, team, matching profile and embedded-JavaScript checks pass with the existing team API key. Apple Development provisioning. |
| Dependency audit | Mobile production dependency audit: 0 vulnerabilities |
| Whitespace | `git diff --check`: pass |

Native and browser fixtures used synthetic hosts, repositories, tracker services
and SSH processes. No real PR/comment/merge, SSH connection, paid provider turn,
physical device permission or application data migration occurred.

Browser acceptance covered these flows:

- GitLab selection, SHA-leased stack operations, Linear/Jira issue intake,
  literal hostile text, private-token clearing, and owner-only native GitHub
  acknowledgement with operation UUID refresh and no resubmission.
- SSH save-before-connect, explicit connect/disconnect, host-key failures,
  multiple hosts, owner-reviewed task wakes, queued/disabled/dismissed events,
  preserved drafts, stale approval reset, guest privacy and visible worker errors.
- History preview invalidation, delayed/out-of-order responses, duplicate-submit
  prevention, frozen file inventories, failed commits with no import, consumed
  previews and separate token metadata.
- Device video with a paused image decoder: only one frame arrived until the
  browser acknowledged it. Stop remained enabled during another pending tool
  command. Resume, stop and revocation worked at 1280/375/320 CSS-pixel widths.
- ACP selector IDs survived save and Edit. Arguments remained literal, hostile
  names rendered as text, and the 375px form had no horizontal overflow.
- Native companion pairing, exact command receipts, draft restoration after app
  termination, read-only states, task owner/guest separation and uncertain task
  delivery. The [native guide](native-mobile-app.md) records the full evidence.

The broad core run exposed a lock-release race while other tests spawned
processes. Explicit last-owner unlock guards now cover history imports, Delivery,
SSH and coordination. The reactive owner independently fixed its corresponding
guard and cancelled-worker ownership. Deterministic inherited-descriptor tests
and the final parallel core suite pass. No retry loop masks ownership failures.

The whole-workspace formatting/gate pass completed after the Whiteboard source
freeze and retained-timing release. It also verified the native restart-fixture
isolation and patched archive/TLS dependencies. The local gate includes mobile
tests and TypeScript and fails with setup instructions if dependencies are absent.
The captured default browser bundle passes 399 local Inbox and 642 synthetic
mail-setup checks; 63 existing Today assertions remain open across stale Apply,
Undo and DST. This does not establish whole-product or live-mail acceptance.

Physical camera/microphone recognition, phone certificate trust and store
distribution remain hardware/release acceptance. Android's verified APK uses a
development key; the iOS archive uses Apple Development provisioning. No App
Store or Play Store release is claimed. Existing authenticated forge/tracker
accounts were not used for write acceptance.

The source-only review pack is generated under
`target/reviews/workbench-mobile-v2/`. Its owned-module patches compare this
pass with the captured starting worktree, whose HEAD was
`c543e865b01ace7b0e29fa110cdc7dfb38aa032d`. Shared integration edits are listed
separately. The pack excludes desktop mobile authority, other thread changes,
credentials and build products; it is not a standalone patch against clean main.
