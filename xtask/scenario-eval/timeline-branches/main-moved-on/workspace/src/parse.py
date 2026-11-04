"""Size parsing."""

SUFFIXES = {"kb": 1024, "mb": 1024 ** 2, "gb": 1024 ** 3}


def parse_size(text):
    """Returns the size in bytes for text like "12", "12kb" or "1mb"."""
    value = text.strip().lower()
    for suffix, factor in SUFFIXES.items():
        if value.endswith(suffix):
            return int(value[: -len(suffix)]) * factor
    return int(value) * 1024
