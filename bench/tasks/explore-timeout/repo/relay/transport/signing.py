"""HMAC signatures for webhook bodies."""

import hashlib
import hmac


def sign(secret, body):
    return hmac.new(secret.encode("utf-8"), body, hashlib.sha256).hexdigest()


def verify(secret, body, signature):
    return hmac.compare_digest(sign(secret, body), signature)
