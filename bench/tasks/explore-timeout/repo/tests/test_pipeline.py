import unittest

from relay.pipeline.runner import run


class PipelineTest(unittest.TestCase):
    def test_filters(self):
        out = []
        messages = [{"body": "a"}, {"body": ""}, {"body": "b", "expires_at": 1}]
        self.assertEqual(run(messages, out.append, now=5), 1)
        self.assertEqual(out, [{"body": "a"}])
