import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from parser.expr import evaluate, parse, tokenize


class ExprTest(unittest.TestCase):
    def test_tokenize(self):
        kinds = [kind for kind, _ in tokenize("12 + (3*4)")]
        self.assertEqual(kinds, ["NUMBER", "OP", "LPAREN", "NUMBER", "OP", "NUMBER", "RPAREN", "END"])

    def test_parse_shape(self):
        self.assertEqual(parse("1 + 2 * 3"), ("add", ("num", 1), ("mul", ("num", 2), ("num", 3))))

    def test_evaluate(self):
        self.assertEqual(evaluate("1 + 2 * 3"), 7)
        self.assertEqual(evaluate("(1 + 2) * 3"), 9)
        self.assertEqual(evaluate("7 / 2"), 3)

    def test_syntax_error(self):
        with self.assertRaises(ValueError):
            parse("1 +")


if __name__ == "__main__":
    unittest.main()
