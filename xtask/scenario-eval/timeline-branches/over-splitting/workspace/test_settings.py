import unittest

from settings import chunk, fits


class SettingsTest(unittest.TestCase):
    def test_chunk(self):
        self.assertEqual(chunk(list(range(12))), [list(range(10)), [10, 11]])

    def test_fits(self):
        self.assertTrue(fits(list(range(10))))
        self.assertFalse(fits(list(range(11))))


if __name__ == "__main__":
    unittest.main()
