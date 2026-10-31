import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from cache.lru import Cache


class Clock:
    def __init__(self):
        self.t = 0.0

    def __call__(self):
        return self.t


class LruTest(unittest.TestCase):
    def test_get_put(self):
        c = Cache(2, 10, Clock())
        c.put("a", 1)
        self.assertEqual(c.get("a"), 1)
        self.assertIsNone(c.get("missing"))

    def test_eviction_is_least_recently_used(self):
        c = Cache(2, 10, Clock())
        c.put("a", 1)
        c.put("b", 2)
        c.get("a")
        c.put("c", 3)
        self.assertIsNone(c.get("b"))
        self.assertEqual(c.get("a"), 1)
        self.assertEqual(len(c), 2)

    def test_expiry(self):
        clock = Clock()
        c = Cache(2, 5, clock)
        c.put("a", 1)
        clock.t = 4.9
        self.assertEqual(c.get("a"), 1)
        clock.t = 5.1
        self.assertIsNone(c.get("a"))
        self.assertEqual(len(c), 0)


if __name__ == "__main__":
    unittest.main()
