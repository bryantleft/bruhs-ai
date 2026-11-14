# Local verification and chat handoffs

Workbench → Verification controls a dedicated browser, connected Android devices and iOS simulators. These tools run on the host computer. They do not provision cloud machines or copy your normal browser profile.

## Allow an agent to verify a change

1. Copy the task or chat's full ID. Enter it in **Task or user** in Verification. A descriptive label such as “Workbench” permits manual controls, but does not give an agent access.
2. Choose **Allow browser and open** or **Allow devices and refresh**. Browser access opens a visible Chrome or Chromium window with a separate temporary profile. Device access discovers installed tools and available targets.
3. The authorized agent receives `local_browser` and `local_device` tools through `bruhs_workbench`. `workspace_status` lists only grants belonging to that thread. An agent cannot create or expand its own grant. Plan/read-only turns do not receive these action tools.
4. Choose **Revoke** to cancel pending actions, close the grant's browser and block subsequent operations. Revocation remains available while another action runs.

Browser actions include navigation, page snapshots, exact-match CSS clicks/fills, JavaScript evaluation, console/network logs and screenshots. Screenshots are saved as ordinary private attachments; the returned absolute path can be embedded in a rich reply. The browser's own local app API access is blocked, so a browser grant does not grant access to private life records.

That boundary covers new windows, frames, dedicated/shared/service workers and WebSocket connections. The browser identifies itself on requests, including the initial service-worker script download. If Chrome rejects or loses its browser protection connection, Bruhs closes the isolated browser and reports the error.

Browser navigation accepts HTTP, HTTPS and `about:blank`. It rejects file URLs and URL credentials. Chrome keeps its browser sandbox enabled. If Chrome is outside the standard install locations, set `BRUHS_BROWSER_BIN` to its executable before starting the host.

Android discovery uses `adb devices -l`; unauthorized and offline devices remain visible with their status. Install SDK Platform-Tools and accept the device's USB debugging prompt. Android supports screenshots, taps, text, Home, Back and opening URLs. Start an emulator through Android Studio before discovery; Bruhs only shuts down Android emulators, not physical devices.

iOS discovery and boot/shutdown/open-URL/screenshots use `xcrun simctl`. Install Xcode and a simulator runtime. Touch, typing and Home additionally use the installed `idb` CLI and companion. iOS has no system Back action; tap the app's back control. Missing tools and failed commands are reported rather than shown as an empty success. Choose **Start live video** after selecting a booted/authorized device. Android
streams `adb exec-out screenrecord --output-format=h264 -` through local ffmpeg;
iOS uses `idb video-stream --format mjpeg --udid <id>`. Install these tools on
the host. `simctl recordVideo` is a finalized recording format and is not used
as a live stream.

Video has one active capture per device grant. Each JPEG stays within the
existing 16 MiB capture budget; the browser acknowledges it only after decoding
before another frame is sent. An unresponsive client reaches the existing local
tool deadline. Stop stays available during other commands. Stop, page closure,
disconnect and revocation kill owned capture process groups and release the
stream. No video is saved. Android may end capture at its OS recorder limit;
restart explicitly to resume. Screenshots remain durable evidence.

## Dictate into a draft

Open composer options and choose **Dictate locally**. Stop inserts text into the draft; Cancel discards the recording. Native dictation replaces the original selected text when the draft is unchanged, and preserves concurrent edits by appending. Thread changes discard pending results; leaving a native chat cancels its recording. Dictation never submits a message.

Recordings stop after five minutes, matching the researched composer dictation contract. Audio uses the capture daemon's local Whisper pipeline and temporary files. Dictation never selects the meeting pipeline's external transcription provider and never creates a meeting. The Verification panel also offers a local model/language picker and an editable draft.

## Continue an existing chat in Workbench

Run **Continue in workbench** from either app's command bar. It copies the existing bounded Markdown export into a new host-owned task, preserves the selected runtime/model and access mode, and keeps the current project/worktree. The original chat remains unchanged. Stop a running source turn first to avoid two agents writing in the same checkout. A missing project or rejected task creation is reported without navigating away.

The new task starts a fresh provider session with the exported context. It does not silently migrate the web's IndexedDB history or the native state file.

## Verification

- `cargo test -p bruhs-models local_tools --lib` checks URL admission and draft insertion.
- `cargo test -p bruhs-models workbench_handoff --lib` checks model, permissions, worktree and context preservation.
- `cargo test -p bruhs-core local_tools --lib` checks grants, revocation, device identity/shell quoting, bounded process output and CDP transport. The browser guard tests cover recursive setup, service-worker acknowledgement ordering and termination after protection fails.
- `cargo test -p bruhs-core live_browser_fills_clicks_and_saves_screenshot_then_revokes --lib -- --ignored` opens a separate visible Chrome window against a synthetic page, checks popup/worker isolation and a synthetic WebSocket handshake, edits the page, captures evidence and revokes access. It closes its own browser and fixture servers.
- `cargo test -p bruhs-capture routes::dictation --bin bruhs-capture` verifies that silent WAV audio creates no invented transcript or meeting.
- `BRUHS_PLAYWRIGHT_PATH=/path/to/playwright node bruhs-web/tests/local_tools_browser.mjs` exercises the real Verification UI with synthetic API responses at desktop, compact and touch widths, including Slow 3G and revocation during a pending action. Captures are stored in `/tmp/bruhs-ui-preview` by default.

Hardware microphone consent, connected devices, local speech recognition accuracy and native visual behavior require their separate live checks. Synthetic fixtures do not establish those outcomes.
