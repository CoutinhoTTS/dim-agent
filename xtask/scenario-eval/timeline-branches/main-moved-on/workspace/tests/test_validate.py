import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from src.validate import valid_username


class ValidUsernameTest(unittest.TestCase):
    def test_accepts_letter_start_with_underscore(self):
        self.assertTrue(valid_username("alice_1"))

    def test_rejects_digit_start(self):
        self.assertFalse(valid_username("1alice"))

    def test_rejects_empty(self):
        self.assertFalse(valid_username(""))

    def test_rejects_long(self):
        self.assertFalse(valid_username("a" * 21))

    def test_rejects_leading_underscore(self):
        self.assertFalse(valid_username("_lead"))


if __name__ == "__main__":
    unittest.main()
