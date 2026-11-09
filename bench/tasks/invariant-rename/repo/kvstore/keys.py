"""Key normalisation.

Keys are case-insensitive and surrounding whitespace is not part of a key.
Every key that is stored, looked up or deleted goes through normalize_key,
and every prefix used to select keys (scans, cache lookups, anything that
matches "keys starting with ...") goes through normalize_prefix, so a caller
that passes "User:" sees the same keys as one that passes "user:".
"""


def normalize_key(key):
    if not isinstance(key, str):
        raise TypeError("keys are strings")
    normalized = key.strip().lower()
    if not normalized:
        raise ValueError("empty key")
    return normalized


def normalize_prefix(prefix):
    # Only lower-cased: a prefix may legitimately end in a space.
    if not isinstance(prefix, str):
        raise TypeError("prefixes are strings")
    return prefix.lower()
