"""Retry helper for outbound calls."""


def call(fn, attempts=3):
    last = None
    for _ in range(attempts):
        try:
            return fn()
        except Exception as error:  # noqa: BLE001
            last = error
    raise last
