"""Log entries and how to apply them.

An entry is a dict:

    {"op": "put", "key": k, "value": v}
    {"op": "delete", "key": k}
    {"op": "batch", "ops": [<put or delete entry>, ...]}   # applied in order

Keys in entries are already normalised.
"""


def apply_entry(data, entry):
    """Apply one log entry to the dict `data` in place."""
    op = entry["op"]
    if op == "put":
        data[entry["key"]] = entry["value"]
    elif op == "delete":
        del data[entry["key"]]
    elif op == "batch":
        for sub in entry["ops"]:
            if sub["op"] == "batch":
                raise ValueError("nested batch")
            apply_entry(data, sub)
    else:
        raise ValueError(f"unknown op {op!r}")
