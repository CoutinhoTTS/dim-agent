"""Recursive-descent parser for integer arithmetic expressions.

Grammar (as the tests expect it):

    expr   := term (("+" | "-") term)*
    term   := factor (("*" | "/") factor)*
    factor := NUMBER | "(" expr ")"

`parse(text)` returns a nested tuple: ("num", n) | ("add", l, r) | ("sub", l, r)
| ("mul", l, r) | ("div", l, r). Whitespace is skipped. A syntax error raises
ValueError with the position in the message.

TODO(open): tokenizer, parse, evaluate.
"""


def tokenize(text):
    """Yields (kind, value) with kind in NUMBER, OP, LPAREN, RPAREN, END."""
    raise NotImplementedError("tokenize")


def parse(text):
    """Returns the parsed expression tree, or raises ValueError."""
    raise NotImplementedError("parse")


def evaluate(text):
    """Parses and evaluates, integer division truncating toward zero."""
    raise NotImplementedError("evaluate")
