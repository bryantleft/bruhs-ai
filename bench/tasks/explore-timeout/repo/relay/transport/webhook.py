"""Webhook deliveries."""

import json

from relay.transport.http_client import HttpClient


class WebhookSender:
    def __init__(self, url, client=None):
        self.url = url
        self.client = client or HttpClient()

    def send(self, message):
        return self.client.post(self.url, json.dumps(message).encode("utf-8"))
