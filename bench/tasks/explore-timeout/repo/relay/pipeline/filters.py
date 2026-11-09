"""Message filters."""


def drop_empty(message):
    return message if message.get("body") else None


def drop_expired(message, now):
    expires = message.get("expires_at")
    return None if expires is not None and expires <= now else message
