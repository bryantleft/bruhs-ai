"""Runs filters, then delivers."""

from relay.pipeline.filters import drop_empty, drop_expired


def run(messages, deliver, now):
    delivered = 0
    for message in messages:
        message = drop_empty(message)
        if message is None:
            continue
        message = drop_expired(message, now)
        if message is None:
            continue
        deliver(message)
        delivered += 1
    return delivered
