"""The HTTP client used for deliveries. Its timeout comes from the
`http.timeout` setting unless one is passed in."""

import urllib.request

from relay.conf import Settings


class HttpClient:
    def __init__(self, settings=None, timeout=None, retries=None):
        settings = settings or Settings()
        self.timeout = timeout if timeout is not None else settings.get("http.timeout")
        self.retries = retries if retries is not None else settings.get("http.retries")
        self.user_agent = settings.get("http.user_agent")

    def build_request(self, url, body):
        request = urllib.request.Request(url, data=body, method="POST")
        request.add_header("User-Agent", self.user_agent)
        request.add_header("Content-Type", "application/json")
        return request

    def post(self, url, body, opener=urllib.request.urlopen):
        last_error = None
        for _ in range(self.retries + 1):
            try:
                with opener(self.build_request(url, body), timeout=self.timeout) as response:
                    return response.status
            except OSError as error:
                last_error = error
        raise last_error
