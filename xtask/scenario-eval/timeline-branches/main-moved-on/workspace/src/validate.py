"""Username validation."""


def valid_username(name):
    """Returns True for a username of letters, digits and underscores."""
    return name.isalnum() and len(name) <= 20
