from config import DEFAULT_TIMEOUT, MAX_RETRIES


def serve(handler):
    for _ in range(MAX_RETRIES):
        try:
            return handler(DEFAULT_TIMEOUT)
        except TimeoutError:
            continue
    raise RuntimeError("gave up")
