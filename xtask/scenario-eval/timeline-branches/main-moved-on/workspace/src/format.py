"""Duration formatting."""


def format_duration(seconds):
    """Returns a duration as "3m05s"."""
    minutes, rest = divmod(seconds, 60)
    return f"{minutes}m{rest}s"
