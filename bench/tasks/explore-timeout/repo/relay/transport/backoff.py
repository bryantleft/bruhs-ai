"""Retry delays."""

MAX_DELAY = 30


def delays(retries, base=0.5):
    """Exponential delays, capped at MAX_DELAY seconds."""
    return [min(MAX_DELAY, base * (2 ** attempt)) for attempt in range(retries)]
