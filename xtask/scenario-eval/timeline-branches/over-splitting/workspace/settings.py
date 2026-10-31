MAX_SIZE = 10
RETRIES = 3


def chunk(items):
    return [items[i:i + MAX_SIZE] for i in range(0, len(items), MAX_SIZE)]


def fits(items):
    return len(items) <= MAX_SIZE
