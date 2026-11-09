The relay's HTTP client gives up on a delivery after 30 seconds by default,
which is too short for some partners. Change the default HTTP request timeout
to 45 seconds.

Only the default changes: a timeout passed to the client or set in a config
file must still win, and none of the other 30-second settings (cache TTL,
database connect timeout, poll interval, retry cap, …) may change. Change the
value where it is defined rather than patching around it.
