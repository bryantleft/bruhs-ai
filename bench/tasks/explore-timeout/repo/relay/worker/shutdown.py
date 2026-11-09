"""Graceful shutdown."""

GRACE_SECONDS = 30


def drain(poller, handle):
    """Handle what is left in the queue before stopping."""
    handled = 0
    while True:
        batch = poller.poll_once()
        if not batch:
            return handled
        for message in batch:
            handle(message)
            handled += 1
