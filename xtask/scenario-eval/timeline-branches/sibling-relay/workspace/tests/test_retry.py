import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "src"))

from retry import call


class RetryTest(unittest.TestCase):
    def test_retries_until_success(self):
        calls = {"n": 0}

        def flaky():
            calls["n"] += 1
            if calls["n"] < 3:
                raise ValueError("boom")
            return "ok"

        self.assertEqual(call(flaky), "ok")
        self.assertEqual(calls["n"], 3)


if __name__ == "__main__":
    unittest.main()
