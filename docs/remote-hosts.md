# Managed local SSH tunnels

Workbench → Hosts saves profiles for the owner's existing OpenSSH aliases or
literal `user@host` destinations. Saving does not connect. Choose the SSH port,
an unprivileged local port, the remote task listener port and an explicit
connection time budget, then press Connect. Profile data stays in the private
local app directory; private keys and passwords are never copied into it.

Bruhs starts foreground OpenSSH with literal arguments, strict known-host
checking, batch authentication and one loopback forward:
`127.0.0.1:local → remote 127.0.0.1:task`. Establish host trust and verify login
using your terminal first. An SSH alias that defines extra forwards is refused
so the profile cannot silently open additional listeners. Existing aliases may
still use their configured identity files, agent and proxy transport.

Connected means the private SSH master acknowledged its own control socket.
It does not prove the remote task app is running or that a pairing grant is
valid. Open the forwarded URL and pair with the remote task host separately.
Only the local Workbench exposes host administration; paired task listeners do
not expose these routes.

Disconnect cancels only the child and process group Bruhs created, including
its proxy descendants on Unix. Bruhs never finds or kills a process using a
saved PID, never stops a user's independent SSH session and never edits SSH
configuration. Normal shutdown requests cancellation. After an interrupted
host, active records become Interrupted and are not reconnected. Inspect a
leftover listener before reconnecting; a hard host crash cannot promise that
every operating system has already reaped every child.

The owner supplies the connection deadline. Status probes reuse the native
mobile connection-status cadence; each probe is a local control-socket command.
Configuration output uses the existing local request budget and stderr retains
the shared CLI diagnostic budget. Profiles are bounded by the existing
50-record visible history capacity. A full page asks the owner to delete an
unused profile. Failed persistence rolls back the public snapshot, blocks new
connections and keeps Disconnect available.

Synthetic subprocess tests cover literal arguments, host-key refusal, extra
forward rejection, process-group cancellation, persistence failure and restart.
They do not connect to an external host or change the user's SSH configuration.
Real network authentication, Windows descendant cleanup and physical phone
network trust require their own environment acceptance.
