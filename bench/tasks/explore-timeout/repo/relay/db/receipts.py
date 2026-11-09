"""Delivery receipts."""

SCHEMA = """
CREATE TABLE IF NOT EXISTS receipts (
    message_id TEXT PRIMARY KEY,
    status INTEGER NOT NULL,
    delivered_at REAL NOT NULL
)
"""


def record(connection, message_id, status, delivered_at):
    connection.execute(SCHEMA)
    connection.execute(
        "INSERT OR REPLACE INTO receipts VALUES (?, ?, ?)",
        (message_id, status, delivered_at),
    )
