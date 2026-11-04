import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from src.format import format_duration


class FormatDurationTest(unittest.TestCase):
    def test_padded_seconds(self):
        self.assertEqual(format_duration(185), "3m05s")

    def test_zero_seconds(self):
        self.assertEqual(format_duration(60), "1m00s")

    def test_under_a_minute(self):
        self.assertEqual(format_duration(5), "0m05s")


if __name__ == "__main__":
    unittest.main()
