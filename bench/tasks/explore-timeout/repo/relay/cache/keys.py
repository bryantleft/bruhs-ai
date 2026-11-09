"""Cache key helpers."""


def route_key(tenant, topic):
    return f"route:{tenant}:{topic}"
