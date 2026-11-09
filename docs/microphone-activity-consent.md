# Microphone activity consent

The capture worker starts idle. It may open the default microphone for a short
volume probe only while an explicit, expiring activity-monitor permission exists.
Probes calculate RMS in memory; they do not save audio or identify a meeting.
This permission is separate from a user-requested recording.

Web and desktop require opt-in for each app session. Legacy persisted
`audio_driven_enabled: true` is reset on startup. Turning off either the activity
switch or the parent auto-record switch cancels the current session. Re-enabling
the parent does not restore microphone permission.

## Local protocol

`GET /activity-monitor` returns `{ epoch, enabled, expires_in_ms, probe_error }`. It does not
grant permission or reveal the owning token. JSON `POST /activity-monitor` accepts:

- `{ operation: "enable", epoch, token }`: explicitly grant a new 30-second
  session; `token` is a client-generated UUID and the epoch must still be current.
  Idle GET requests offer a fresh challenge that expires after 30 seconds. An
  initial grant uses that original deadline rather than extending it when a
  delayed enable arrives. A newer idle challenge invalidates older ones.
- `{ operation: "renew", epoch, token }`: extend only the current owner's live
  session. Clients renew every 10 seconds, off the interaction thread.
- `{ operation: "disable", epoch }`: revoke the epoch, including an in-flight
  enable request. The response waits for the active probe to release its device;
  a three-second shutdown timeout returns an error, not false confirmation.

Expiry, revocation, and daemon restart rotate the epoch. A late enable or renewal
cannot resurrect an expired/revoked epoch. Permission is not saved to disk. A
closed, crashed, suspended, or disconnected client cannot renew indefinitely;
checks stop when the last permission expires. Normal probe cancellation is
checked at most every 20 ms while sampling. This is an implementation polling
bound, not a measured hardware shutdown guarantee.

Clients record the epoch before sending enable, so cancellation can revoke a
grant even if the enable response is lost. Request deadlines are five seconds.
Late shutdown feedback is fenced so it cannot overwrite a newer session's status.
Failures reset the desired switch and explain how to retry. Unknown shutdown is
disclosed rather than displayed as confirmed off.
Device/permission probe errors belong to the current lease and are returned on
renewal. Both clients revoke the grant, reset the switch, and explain the input
device/microphone-access check instead of indefinitely displaying healthy
monitoring. A stale probe cannot attach an error to a newer session.

The daemon remains bound to loopback. Its production router rejects foreign
Origins and non-loopback Host values before dispatching handlers; CORS alone
would not prevent simple cross-origin writes. Allowed browser origins are HTTP(S)
on `localhost`, valid `*.localhost` names, `127.0.0.1`, or `[::1]`. Native clients
without an Origin are supported. This is browser-origin isolation, not
authentication against other processes running as the same local user.

## Testing

All tests are synthetic. Worker tests inject a sampler; production-router HTTP
tests never spawn the hardware worker. Shared tests exercise cancellation,
ownership, and stale responses. GPUI tests check legacy-preference reset and
parent-switch cancellation without network I/O. The browser runner is
`scripts/verify-activity-monitor.playwright.mjs`; its only permitted writes go to
a fake lease endpoint.

The UI and the capture daemon must be updated together: an older daemon does
not implement this permission protocol.
