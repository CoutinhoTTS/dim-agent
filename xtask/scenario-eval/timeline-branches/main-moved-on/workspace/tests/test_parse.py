import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from src.parse import parse_size


class ParseSizeTest(unittest.TestCase):
    def test_plain_number_is_bytes(self):
        self.assertEqual(parse_size("12"), 12)

    def test_kilobytes(self):
        self.assertEqual(parse_size("12kb"), 12 * 1024)

    def test_megabytes(self):
        self.assertEqual(parse_size("3mb"), 3 * 1024 ** 2)

    def test_case_and_space(self):
        self.assertEqual(parse_size(" 2 GB "), 2 * 1024 ** 3)


if __name__ == "__main__":
    unittest.main()
